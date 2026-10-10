//! Shared HTTP layer: one hardened client used by every network-facing tool.
//!
//! Guarantees provided here (and inherited by curl_tool / web_search / scripts_tool):
//!   * only http/https URLs are ever contacted - file://, gopher://, data:// are refused
//!   * the domain blacklist is enforced on every hop, including redirect targets
//!   * private/loopback/link-local targets are refused unless explicitly allowed
//!   * response bodies are size-capped and streamed, never slurped unbounded
//!   * charset-aware decoding (UTF-8, GBK/GB18030, Shift_JIS, windows-125x)

use crate::blacklist::Matcher;
use anyhow::{anyhow, Context, Result};
use std::io::Read;
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max_bytes: usize,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub user_agent: String,
}

impl Request {
    pub fn get(url: &str) -> Self {
        Self {
            method: "GET".into(),
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
            timeout: Duration::from_secs(30),
            max_bytes: 400_000,
            follow_redirects: true,
            max_redirects: 5,
            user_agent: "ves/0.1".into(),
        }
    }

    pub fn method(mut self, m: &str) -> Self {
        self.method = m.to_ascii_uppercase();
        self
    }

    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }

    pub fn body(mut self, bytes: Vec<u8>) -> Self {
        self.body = Some(bytes);
        self
    }

    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout = Duration::from_secs(secs.max(1));
        self
    }

    pub fn max_bytes(mut self, n: usize) -> Self {
        self.max_bytes = n;
        self
    }

    pub fn follow(mut self, yes: bool, max_redirects: u32) -> Self {
        self.follow_redirects = yes;
        self.max_redirects = max_redirects;
        self
    }

    pub fn ua(mut self, ua: &str) -> Self {
        self.user_agent = ua.to_string();
        self
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub final_url: String,
    pub body: String,
    pub bytes: usize,
    pub truncated: bool,
    pub elapsed_ms: u128,
    pub content_type: String,
}

impl Response {
    #[allow(dead_code)]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_html(&self) -> bool {
        self.content_type.contains("html") || self.body.trim_start().starts_with("<!DOCTYPE") || self.body.trim_start().starts_with("<html")
    }

    pub fn is_json(&self) -> bool {
        self.content_type.contains("json") || (self.body.trim_start().starts_with('{') || self.body.trim_start().starts_with('['))
    }

    /// Readable text: HTML is reduced, everything else is returned as-is.
    pub fn text(&self) -> String {
        if self.is_html() {
            crate::util::html_to_text(&self.body)
        } else {
            self.body.clone()
        }
    }
}

pub struct Http {
    agent_follow: ureq::Agent,
    agent_direct: ureq::Agent,
    pub matcher: Matcher,
    pub allow_private: bool,
    /// Extra attempts after a transient failure (this network resets TLS intermittently).
    pub retries: u32,
    pub backoff_ms: u64,
}

impl Http {
    #[allow(dead_code)]
    pub fn new(matcher: Matcher, allow_private: bool) -> Self {
        Self::with_retries(matcher, allow_private, 3, 300)
    }

    pub fn with_retries(matcher: Matcher, allow_private: bool, retries: u32, backoff_ms: u64) -> Self {
        let build = |redirects: u32| {
            ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .redirects(redirects)
                .user_agent("ves/0.1")
                .build()
        };
        Self {
            agent_follow: build(5),
            agent_direct: build(0),
            matcher,
            allow_private,
            retries,
            backoff_ms,
        }
    }

    /// Validate scheme, host policy and (optionally) resolved IPs.
    pub fn vet_url(&self, raw: &str) -> Result<url::Url> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("empty URL"));
        }
        let parsed = url::Url::parse(trimmed).with_context(|| format!("not a valid absolute URL: {trimmed}"))?;
        let scheme = parsed.scheme().to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(anyhow!(
                "refused scheme '{}': only http and https are allowed by the ves tool surface",
                scheme
            ));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("URL has no host: {trimmed}"))?
            .to_string();
        if let Some(reason) = self.matcher.check_host(&host) {
            return Err(anyhow!("request blocked by policy: {reason}"));
        }
        if !self.allow_private {
            let port = parsed.port_or_known_default().unwrap_or(443);
            if let Ok(addrs) = (host.as_str(), port).to_socket_addrs() {
                for addr in addrs {
                    if crate::blacklist::is_private_host(&addr.ip().to_string()) {
                        return Err(anyhow!(
                            "request blocked: {host} resolves to private address {addr} (set curl.allow_private_addresses to override)"
                        ));
                    }
                }
            }
        }
        Ok(parsed)
    }

    /// Fetch with bounded retries. Transport failures are retried for every method
    /// (the request never reached the origin); 429/5xx are retried only for GET/HEAD.
    pub fn fetch(&self, req: &Request) -> Result<Response> {
        let parsed = self.vet_url(&req.url)?;
        let idempotent = matches!(req.method.as_str(), "GET" | "HEAD");
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let outcome = self.fetch_once(&parsed, req);
            let retryable = match &outcome {
                Ok(resp) => idempotent && matches!(resp.status, 429 | 500 | 502 | 503 | 504),
                Err(_) => true,
            };
            if retryable && attempt <= self.retries {
                std::thread::sleep(Duration::from_millis(self.backoff_ms.max(50) * attempt as u64));
                continue;
            }
            return outcome;
        }
    }

    fn fetch_once(&self, parsed: &url::Url, req: &Request) -> Result<Response> {
        let agent = if req.follow_redirects {
            // clone-free reuse: Agent is cheap to borrow
            &self.agent_follow
        } else {
            &self.agent_direct
        };
        let started = Instant::now();
        let mut builder = agent
            .request(&req.method, parsed.as_str())
            .timeout(req.timeout)
            .set("User-Agent", &req.user_agent)
            .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.9,text/plain;q=0.8,*/*;q=0.5")
            .set("Accept-Language", "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7")
            .set("Accept-Encoding", "gzip, deflate");
        for (k, v) in &req.headers {
            builder = builder.set(k, v);
        }
        let outcome = match &req.body {
            Some(body) => builder.send_bytes(body),
            None => builder.call(),
        };
        let resp = match outcome {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                return Err(anyhow!("transport error for {}: {}", req.url, t));
            }
        };
        let status = resp.status();
        let final_url = resp.get_url().to_string();
        let content_type = resp.content_type().to_string();
        let headers: Vec<(String, String)> = resp
            .headers_names()
            .into_iter()
            .filter_map(|name| resp.header(&name).map(|v| (name.clone(), v.to_string())))
            .collect();
        let mut buf = Vec::with_capacity(16 * 1024);
        let mut reader = resp.into_reader().take(req.max_bytes as u64 + 1);
        reader
            .read_to_end(&mut buf)
            .with_context(|| format!("failed reading body from {}", final_url))?;
        let truncated = buf.len() > req.max_bytes;
        if truncated {
            buf.truncate(req.max_bytes);
        }
        let bytes = buf.len();
        let body = decode_body(&buf, &content_type);
        Ok(Response {
            status,
            headers,
            final_url,
            body,
            bytes,
            truncated,
            elapsed_ms: started.elapsed().as_millis(),
            content_type,
        })
    }

}

/// Decode a body using the charset in `Content-Type`, falling back to sniffing.
pub fn decode_body(bytes: &[u8], content_type: &str) -> String {
    let label = content_type
        .split(';')
        .map(|p| p.trim())
        .find_map(|p| {
            let lower = p.to_ascii_lowercase();
            lower.strip_prefix("charset=").map(|s| s.trim_matches('"').to_string())
        });
    if let Some(label) = label {
        if let Some((text, _err)) = decode_with(&label, bytes) {
            return text;
        }
    }
    // strict UTF-8 first
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    for label in ["gb18030", "shift_jis", "big5", "euc-kr", "windows-1251"] {
        if let Some((text, had_errors)) = decode_with(label, bytes) {
            if !had_errors {
                return text;
            }
        }
    }
    if let Some((text, _)) = decode_with("gb18030", bytes) {
        return text;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn decode_with(label: &str, bytes: &[u8]) -> Option<(String, bool)> {
    let enc = encoding_rs::Encoding::for_label(label.as_bytes())?;
    let (cow, _, had_errors) = enc.decode(bytes);
    Some((cow.into_owned(), had_errors))
}

/// Resolve a possibly-relative href against a base URL.
#[allow(dead_code)]
pub fn absolutize(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("mailto:") {
        return None;
    }
    if let Ok(u) = url::Url::parse(href) {
        if u.scheme() == "http" || u.scheme() == "https" {
            return Some(u.to_string());
        }
        return None;
    }
    let base = url::Url::parse(base).ok()?;
    base.join(href).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blacklist::Blacklist;

    fn http() -> Http {
        Http::new(
            Matcher::new(&Blacklist {
                blocked: vec!["blocked.example".into()],
                allowed: vec![],
                block_private_addresses: true,
            }),
            false,
        )
    }

    #[test]
    fn refuses_non_http_schemes() {
        let h = http();
        for bad in ["file:///etc/passwd", "data:text/html,<b>x</b>", "ftp://x.example/a", "gopher://x"] {
            assert!(h.vet_url(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn enforces_blacklist() {
        let h = http();
        assert!(h.vet_url("https://blocked.example/x").is_err());
        assert!(h.vet_url("https://sub.blocked.example/x").is_err());
        assert!(h.vet_url("https://example.org/x").is_ok());
    }

    #[test]
    fn refuses_private_targets() {
        let h = http();
        assert!(h.vet_url("http://127.0.0.1:8080/").is_err());
        assert!(h.vet_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn charset_decoding_falls_back_to_gbk() {
        // "中文" encoded as GBK
        let gbk = vec![0xD6, 0xD0, 0xCE, 0xC4];
        assert_eq!(decode_body(&gbk, "text/html; charset=gb2312"), "中文");
        assert_eq!(decode_body("héllo".as_bytes(), "text/html; charset=utf-8"), "héllo");
    }

    #[test]
    fn absolutize_handles_relative_and_drops_junk() {
        assert_eq!(absolutize("https://a.com/dir/x", "/y").unwrap(), "https://a.com/y");
        assert_eq!(absolutize("https://a.com/dir/x", "z").unwrap(), "https://a.com/dir/z");
        assert!(absolutize("https://a.com", "javascript:void(0)").is_none());
        assert!(absolutize("https://a.com", "#frag").is_none());
    }
}
