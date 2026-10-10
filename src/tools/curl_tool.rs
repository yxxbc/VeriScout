//! curl_tool - a hardened, shell-free HTTP client.
//!
//! Two engines exist:
//!   * native (default): ureq-based in-process HTTP. No subprocess, no filesystem,
//!     no way to reach a shell; the argument surface cannot express local file access.
//!   * binary (opt-in): the system curl binary, invoked with an argv that this module
//!     builds from validated fields only. User input never reaches a shell and only
//!     allowlisted flags are used, so there is no injection path.

use super::{Args, Ctx};
use crate::http::Request;
use crate::util;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::process::Command;

const MAX_BODY_BYTES: usize = 1_000_000;
const ALLOWED_METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];

pub fn run(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = Args::new(args);
    let url = a.req_str("url")?;
    let method = a.str("method").unwrap_or_else(|| "GET".into()).to_ascii_uppercase();
    if !ALLOWED_METHODS.contains(&method.as_str()) {
        return Err(anyhow!(
            "method '{method}' is not allowed; permitted: {}",
            ALLOWED_METHODS.join(", ")
        ));
    }
    let follow = a.bool("follow_redirects", ctx.cfg.curl.follow_redirects);
    let timeout = a.u64("timeout_secs", ctx.cfg.curl.timeout_secs).clamp(1, 120);
    let max_bytes = (a.u64("max_bytes", ctx.cfg.curl.max_bytes as u64) as usize).clamp(1024, 8_000_000);
    let extract_text = a.bool("extract_text", true);
    let render = a.bool("render", false);
    let include_headers = a.bool("include_headers", false);
    let save = a.bool("save", false);
    let headers = a.object("headers");

    // --- body assembly -----------------------------------------------------
    let mut body_bytes: Option<Vec<u8>> = None;
    let mut content_type: Option<String> = None;
    if let Some(v) = args.get("json") {
        if !v.is_null() {
            let bytes = serde_json::to_vec(v)?;
            if bytes.len() > MAX_BODY_BYTES {
                return Err(anyhow!("JSON body too large ({} bytes, max {MAX_BODY_BYTES})", bytes.len()));
            }
            body_bytes = Some(bytes);
            content_type = Some("application/json".into());
        }
    }
    if let Some(raw) = a.str("body") {
        if !raw.is_empty() {
            if raw.len() > MAX_BODY_BYTES {
                return Err(anyhow!("request body too large ({} bytes, max {MAX_BODY_BYTES})", raw.len()));
            }
            body_bytes = Some(raw.into_bytes());
            content_type = content_type.or_else(|| Some("text/plain; charset=utf-8".into()));
        }
    }

    // --- policy vetting before anything leaves the process ------------------
    let parsed = match ctx.http.vet_url(&url) {
        Ok(u) => u,
        Err(e) => {
            ctx.session.blocked_requests.push(format!("{url}: {e}"));
            return Err(e);
        }
    };

    // Optional: hand the URL to the AnySearch extractor, which renders JS-heavy pages
    // and returns Markdown. Same policy gates first, so blocked/private URLs never leave.
    if render {
        if !ctx.cfg.search.anysearch_extract {
            return Err(anyhow!("render=true is disabled (search.anysearch_extract=false)"));
        }
        let markdown = crate::search::providers::anysearch::extract(&ctx.cfg, parsed.as_str())
            .map_err(|e| anyhow!("render failed: {e}"))?;
        ctx.session.fetched_urls.push(parsed.to_string());
        let (text, text_truncated) = util::truncate_chars(&markdown, 60_000);
        return Ok(json!({
            "status": 200,
            "url": parsed.as_str(),
            "content_type": "text/markdown",
            "engine": "anysearch-extract",
            "extractor": "anysearch",
            "bytes": markdown.len(),
            "truncated": text_truncated,
            "text": text,
            "note": "rendered by the AnySearch extractor (third-party renderer); still verify against a primary source",
        }));
    }

    let use_binary = ctx.cfg.curl.engine.eq_ignore_ascii_case("binary");
    let raw_body = if use_binary {
        binary_fetch(ctx, &parsed, &method, &headers, content_type.as_deref(), body_bytes, timeout, max_bytes, follow)?
    } else {
        let mut req = Request::get(parsed.as_str())
            .method(&method)
            .timeout(timeout)
            .max_bytes(max_bytes)
            .follow(follow, ctx.cfg.curl.max_redirects)
            .ua(&ctx.cfg.curl.user_agent);
        for (k, v) in &headers {
            req = req.header(k, v);
        }
        if let Some(ct) = &content_type {
            req = req.header("Content-Type", ct);
        }
        if let Some(b) = body_bytes {
            req = req.body(b);
        }
        let resp = ctx.http.fetch(&req)?;
        Fetched {
            status: resp.status,
            final_url: resp.final_url,
            content_type: resp.content_type,
            headers: resp.headers,
            body: resp.body,
            bytes: resp.bytes,
            truncated: resp.truncated,
            elapsed_ms: resp.elapsed_ms,
        }
    };

    ctx.session.fetched_urls.push(raw_body.final_url.clone());

    let text = if extract_text && raw_body.content_type.contains("html") {
        util::html_to_text(&raw_body.body)
    } else if extract_text && (raw_body.content_type.contains("json") || looks_like_json(&raw_body.body)) {
        pretty_json(&raw_body.body).unwrap_or_else(|| raw_body.body.clone())
    } else {
        raw_body.body.clone()
    };
    let (text, text_truncated) = util::truncate_chars(&text, if extract_text { 40_000 } else { 60_000 });

    let mut out = json!({
        "status": raw_body.status,
        "url": raw_body.final_url,
        "content_type": raw_body.content_type,
        "bytes": raw_body.bytes,
        "truncated": raw_body.truncated,
        "text_truncated": text_truncated,
        "elapsed_ms": raw_body.elapsed_ms,
        "engine": if use_binary { "binary" } else { "native" },
        "text": text,
    });
    if include_headers {
        out["headers"] = json!(raw_body.headers.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>());
    }
    if save {
        let path = save_raw(ctx, &raw_body.final_url, &raw_body.body, &raw_body.content_type)?;
        out["saved_path"] = json!(path);
    }
    if raw_body.status == 404 {
        out["note"] = json!("404: the page does not exist - do not cite it");
    } else if raw_body.status == 403 || raw_body.status == 429 {
        out["note"] = json!("The site refused automated access; rely on search snippets or another source.");
    }
    Ok(out)
}

struct Fetched {
    status: u16,
    final_url: String,
    content_type: String,
    headers: Vec<(String, String)>,
    body: String,
    bytes: usize,
    truncated: bool,
    elapsed_ms: u128,
}

fn looks_like_json(body: &str) -> bool {
    let t = body.trim_start();
    t.starts_with('{') || t.starts_with('[')
}

fn pretty_json(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    serde_json::to_string_pretty(&v).ok()
}

fn save_raw(ctx: &Ctx, url: &str, body: &str, content_type: &str) -> Result<String> {
    let dir = ctx.cfg.raw_dir();
    std::fs::create_dir_all(&dir)?;
    let ext = if content_type.contains("json") {
        "json"
    } else if content_type.contains("html") {
        "html"
    } else if content_type.contains("xml") {
        "xml"
    } else {
        "txt"
    };
    let name = format!("{}-{}.{}", util::slugify(url), &util::short_hash(url)[..6], ext);
    let path = dir.join(&name);
    // the path is constructed from a slug + hash, so it cannot escape the raw directory
    std::fs::write(&path, body)?;
    Ok(path.display().to_string())
}

/// Invoke the system curl with a flag whitelist built wholly from validated fields.
#[allow(clippy::too_many_arguments)]
fn binary_fetch(
    ctx: &Ctx,
    parsed: &url::Url,
    method: &str,
    headers: &[(String, String)],
    content_type: Option<&str>,
    body: Option<Vec<u8>>,
    timeout: u64,
    max_bytes: usize,
    follow: bool,
) -> Result<Fetched> {
    let bin = if ctx.cfg.curl.binary_path.trim().is_empty() {
        "curl"
    } else {
        ctx.cfg.curl.binary_path.trim()
    };
    let tmp = std::env::temp_dir().join(format!("ves-curl-{}", &util::short_hash(parsed.as_str())[..8]));
    std::fs::create_dir_all(&tmp)?;
    let body_file = tmp.join("body.bin");
    let header_file = tmp.join("headers.txt");
    let mut cmd = Command::new(bin);
    cmd.arg("-sS")
        .arg("--max-time")
        .arg(timeout.to_string())
        .arg("--max-redirs")
        .arg(ctx.cfg.curl.max_redirects.to_string())
        .arg("--user-agent")
        .arg(&ctx.cfg.curl.user_agent)
        .arg("-X")
        .arg(method)
        .arg("-o")
        .arg(&body_file)
        .arg("-D")
        .arg(&header_file)
        .arg("-w")
        .arg("%{http_code}|%{url_effective}|%{content_type}|%{size_download}|%{time_total}")
        .arg("--proto")
        .arg("=http,https")
        .arg("--max-filesize")
        .arg(max_bytes.to_string());
    if follow {
        cmd.arg("-L");
    }
    if let Some(ct) = content_type {
        cmd.arg("-H").arg(format!("Content-Type: {ct}"));
    }
    for (k, v) in headers {
        // reject header injection
        if k.contains('\n') || k.contains('\r') || v.contains('\n') || v.contains('\r') {
            return Err(anyhow!("header values may not contain newlines"));
        }
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    if let Some(b) = body {
        let payload = tmp.join("payload.bin");
        std::fs::write(&payload, &b)?;
        cmd.arg("--data-binary").arg(format!("@{}", payload.display()));
    }
    cmd.arg("--").arg(parsed.as_str());
    let output = cmd
        .output()
        .map_err(|e| anyhow!("cannot run curl binary '{bin}': {e} (set curl.engine to \"native\" to avoid the subprocess)"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(anyhow!("curl failed: {stderr}"));
    }
    let stats = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut parts = stats.split('|');
    let status: u16 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let final_url = parts.next().unwrap_or(parsed.as_str()).to_string();
    let content_type = parts.next().unwrap_or("").to_string();
    let mut bytes_buf = Vec::new();
    let mut f = std::fs::File::open(&body_file)?;
    f.read_to_end(&mut bytes_buf)?;
    let truncated = bytes_buf.len() > max_bytes;
    if truncated {
        bytes_buf.truncate(max_bytes);
    }
    let body_text = crate::http::decode_body(&bytes_buf, &content_type);
    let headers_out: Vec<(String, String)> = std::fs::read_to_string(&header_file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().to_ascii_lowercase().starts_with("http/") {
                None
            } else {
                Some((k.trim().to_string(), v.trim().to_string()))
            }
        })
        .collect();
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(Fetched {
        status,
        final_url,
        content_type,
        headers: headers_out,
        body: body_text,
        bytes: bytes_buf.len(),
        truncated,
        elapsed_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn ctx() -> Ctx {
        Ctx::new(Config::default())
    }

    #[test]
    fn rejects_local_schemes_and_private_hosts() {
        let mut c = ctx();
        assert!(run(&mut c, &json!({"url": "file:///etc/passwd"})).is_err());
        assert!(run(&mut c, &json!({"url": "http://127.0.0.1:9200/"})).is_err());
        assert!(run(&mut c, &json!({"url": "file:///etc/passwd", "render": true})).is_err());
        assert!(!c.session.blocked_requests.is_empty());
    }

    #[test]
    fn rejects_dangerous_methods() {
        let mut c = ctx();
        let err = run(&mut c, &json!({"url": "https://example.com", "method": "CONNECT"})).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn requires_url() {
        let mut c = ctx();
        assert!(run(&mut c, &json!({})).unwrap_err().to_string().contains("missing required"));
    }
}
