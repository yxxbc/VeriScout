//! General-purpose engines: Bing, Wikipedia, DuckDuckGo, SearxNG, Mojeek.

use super::{between, clean, enc, enc_path, fetch, json_at, mk};
use crate::config::Config;
use crate::http::Http;
use crate::search::SearchResult;
use crate::util;
use anyhow::{anyhow, Result};
use regex::Regex;

// ---------------------------------------------------------------------------
// Bing (HTML endpoint; no API key required)
// ---------------------------------------------------------------------------

pub fn bing(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let lang = if util::contains_cjk(q) { "zh-Hans" } else { "en" };
    let url = format!(
        "https://www.bing.com/search?q={}&count={}&setlang={}&form=QBLH",
        enc(q),
        (limit * 2).min(30),
        lang
    );
    let resp = fetch(http, cfg, &url, &[("Cookie", "SRCHHPGUSR=SRCHLANG=zh-Hans; _EDGE_S=mkt=zh-cn")])?;
    if !resp.is_html() {
        return Err(anyhow!("bing returned {} ({}), not HTML", resp.status, resp.content_type));
    }
    let re = Regex::new(r#"(?s)<h2[^>]*>[^<]*<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)?;
    let re_snip = Regex::new(r#"(?s)<p class="[^"]*b_lineclamp[^"]*"[^>]*>(.*?)</p>"#)?;
    let re_any_p = Regex::new(r#"(?s)<p[^>]*>(.*?)</p>"#)?;
    let mut out = Vec::new();
    for block in resp.body.split("class=\"b_algo\"").skip(1) {
        let Some(caps) = re.captures(block) else { continue };
        let raw_url = unwrap_redirect(&caps[1]);
        let title = clean(&caps[2]);
        if title.is_empty() || !raw_url.starts_with("http") {
            continue;
        }
        let snippet = re_snip
            .captures(block)
            .or_else(|| re_any_p.captures(block))
            .map(|c| clean(&c[1]))
            .unwrap_or_default();
        let mut r = mk("bing", &title, &raw_url, &snippet);
        if r.domain.ends_with("bing.com") {
            continue;
        }
        r.published = block
            .find("class=\"news_dt\"")
            .and_then(|_| between(block, "class=\"news_dt\">", "<").map(|s| s.to_string()));
        out.push(r);
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!("bing returned a page with no parseable results (possible bot interstitial)"));
    }
    Ok(out)
}

/// Bing wraps outbound links in /ck/a?...&u=a1<base64url>
pub fn unwrap_redirect(url: &str) -> String {
    let url = util::decode_entities(url);
    if let Ok(parsed) = url::Url::parse(&url) {
        let is_wrapper = parsed
            .host_str()
            .map(|h| h.ends_with("bing.com") && parsed.path().starts_with("/ck/a"))
            .unwrap_or(false);
        if is_wrapper {
            for (k, v) in parsed.query_pairs() {
                if k == "u" {
                    let trimmed = v.trim_start_matches("a1");
                    if let Some(bytes) = util::base64url_decode(trimmed) {
                        if let Ok(s) = String::from_utf8(bytes) {
                            if s.starts_with("http") {
                                return s;
                            }
                        }
                    }
                }
            }
        }
        if parsed.host_str().map(|h| h.ends_with("duckduckgo.com")).unwrap_or(false) {
            if let Some(uddg) = parsed.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.to_string()) {
                return util::percent_decode(&uddg);
            }
        }
    }
    url
}

// ---------------------------------------------------------------------------
// Wikipedia (MediaWiki search API; strong for entity / factual verification)
// ---------------------------------------------------------------------------

pub fn wikipedia(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let langs: [&str; 2] = if util::contains_cjk(q) { ["zh", "en"] } else { ["en", "zh"] };
    let mut out = Vec::new();
    let mut last_err: Option<String> = None;
    for lang in langs {
        let api = format!(
            "https://{lang}.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit={}&format=json&utf8=1&srprop=snippet|timestamp",
            enc(q),
            limit
        );
        match fetch(http, cfg, &api, &[]) {
            Ok(resp) => {
                let v: serde_json::Value = match serde_json::from_str(&resp.body) {
                    Ok(v) => v,
                    Err(e) => {
                        last_err = Some(format!("wikipedia({lang}) JSON parse failed: {e}"));
                        continue;
                    }
                };
                let Some(items) = json_at(&v, &["query", "search"]).and_then(|x| x.as_array()) else {
                    last_err = Some(format!("wikipedia({lang}) returned no search array"));
                    continue;
                };
                for item in items {
                    let title = item.get("title").and_then(|t| t.as_str()).unwrap_or_default();
                    if title.is_empty() {
                        continue;
                    }
                    let snippet = item.get("snippet").and_then(|s| s.as_str()).unwrap_or_default();
                    let url = format!("https://{lang}.wikipedia.org/wiki/{}", enc_path(title));
                    let mut r = mk("wikipedia", title, &url, snippet);
                    r.extra = Some(format!("wikipedia:{lang}"));
                    r.published = item.get("timestamp").and_then(|t| t.as_str()).map(|s| s.to_string());
                    out.push(r);
                }
            }
            Err(e) => last_err = Some(format!("wikipedia({lang}): {e}")),
        }
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!(last_err.unwrap_or_else(|| "wikipedia returned nothing".into())));
    }
    out.truncate(limit);
    Ok(out)
}

// ---------------------------------------------------------------------------
// DuckDuckGo (html + lite endpoints, then the Instant Answer API)
// ---------------------------------------------------------------------------

pub fn duckduckgo(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let mut errors = Vec::new();

    let html_url = format!("https://html.duckduckgo.com/html/?q={}&kl=wt-wt", enc(q));
    match fetch(http, cfg, &html_url, &[]) {
        Ok(resp) => {
            let re_a = Regex::new(r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)?;
            let re_a2 = Regex::new(r#"(?s)<a[^>]*href="([^"]+)"[^>]*class="result__a"[^>]*>(.*?)</a>"#)?;
            let re_snip = Regex::new(r#"(?s)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#)?;
            let mut out = Vec::new();
            for caps in re_a.captures_iter(&resp.body).chain(re_a2.captures_iter(&resp.body)) {
                let url = unwrap_redirect(&caps[1]);
                if !url.starts_with("http") {
                    continue;
                }
                let snippet = re_snip
                    .captures_iter(&resp.body)
                    .nth(out.len())
                    .map(|c| clean(&c[1]))
                    .unwrap_or_default();
                out.push(mk("duckduckgo", &clean(&caps[2]), &url, &snippet));
                if out.len() >= limit {
                    break;
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
            errors.push("duckduckgo html endpoint returned no results".to_string());
        }
        Err(e) => errors.push(format!("duckduckgo html: {e}")),
    }

    let lite_url = format!("https://lite.duckduckgo.com/lite/?q={}", enc(q));
    if let Ok(resp) = fetch(http, cfg, &lite_url, &[]) {
        let re_link = Regex::new(r#"(?s)<a[^>]*class="result-link"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)?;
        let re_snip = Regex::new(r#"(?s)<td[^>]*class="result-snippet"[^>]*>(.*?)</td>"#)?;
        let mut out = Vec::new();
        for caps in re_link.captures_iter(&resp.body) {
            let url = unwrap_redirect(&caps[1]);
            if !url.starts_with("http") {
                continue;
            }
            let snippet = re_snip
                .captures_iter(&resp.body)
                .nth(out.len())
                .map(|c| clean(&c[1]))
                .unwrap_or_default();
            out.push(mk("duckduckgo", &clean(&caps[2]), &url, &snippet));
            if out.len() >= limit {
                break;
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
        errors.push("duckduckgo lite endpoint returned no results".to_string());
    }

    // Instant Answer API: no key, useful for entity resolution and disambiguation.
    let ia_url = format!("https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1", enc(q));
    if let Ok(resp) = fetch(http, cfg, &ia_url, &[]) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp.body) {
            let mut out = Vec::new();
            if let (Some(text), Some(url)) = (
                v.get("AbstractText").and_then(|x| x.as_str()),
                v.get("AbstractURL").and_then(|x| x.as_str()),
            ) {
                if !text.is_empty() && url.starts_with("http") {
                    let title = v.get("Heading").and_then(|x| x.as_str()).unwrap_or(url);
                    let mut r = mk("duckduckgo", title, url, text);
                    r.extra = Some(format!("instant-answer: {}", v.get("AbstractSource").and_then(|x| x.as_str()).unwrap_or("")));
                    out.push(r);
                }
            }
            if let Some(topics) = v.get("RelatedTopics").and_then(|x| x.as_array()) {
                for t in topics {
                    let (text, url) = match t.get("Topics").and_then(|x| x.as_array()) {
                        Some(nested) => nested
                            .iter()
                            .find_map(|n| Some((n.get("Text")?.as_str()?.to_string(), n.get("FirstURL")?.as_str()?.to_string())))
                            .unwrap_or_default(),
                        None => (
                            t.get("Text").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                            t.get("FirstURL").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                        ),
                    };
                    if url.starts_with("http") {
                        let title: String = text.split(" - ").next().unwrap_or(&text).chars().take(120).collect();
                        out.push(mk("duckduckgo", &title, &url, &text));
                    }
                    if out.len() >= limit {
                        break;
                    }
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
        errors.push("duckduckgo instant answer returned nothing usable".to_string());
    }

    Err(anyhow!(errors.join("; ")))
}

// ---------------------------------------------------------------------------
// SearxNG (self-hosted metasearch; instances rotate, JSON then HTML)
// ---------------------------------------------------------------------------

pub fn searx(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let mut errors = Vec::new();
    for instance in &cfg.search.searx_instances {
        let base = instance.trim().trim_end_matches('/');
        if base.is_empty() {
            continue;
        }
        let json_url = format!("{base}/search?q={}&format=json&language=auto&safesearch=0&pageno=1", enc(q));
        match fetch(http, cfg, &json_url, &[]) {
            Ok(resp) if resp.is_json() => {
                let v: serde_json::Value = serde_json::from_str(&resp.body)?;
                let mut out = Vec::new();
                if let Some(items) = v.get("results").and_then(|x| x.as_array()) {
                    for item in items {
                        let url = item.get("url").and_then(|x| x.as_str()).unwrap_or_default();
                        let title = item.get("title").and_then(|x| x.as_str()).unwrap_or_default();
                        if url.is_empty() || title.is_empty() {
                            continue;
                        }
                        let content = item.get("content").and_then(|x| x.as_str()).unwrap_or_default();
                        let mut r = mk("searx", title, url, content);
                        r.published = item.get("publishedDate").and_then(|x| x.as_str()).map(|s| s.to_string());
                        r.extra = Some(format!("instance:{base}"));
                        out.push(r);
                        if out.len() >= limit {
                            break;
                        }
                    }
                }
                if !out.is_empty() {
                    return Ok(out);
                }
                errors.push(format!("{base}: json responded without results"));
            }
            Ok(resp) => {
                // HTML fallback
                let re = Regex::new(r#"(?s)<a[^>]*href="(https?://[^"]+)"[^>]*class="url_[^"]*"[^>]*>(.*?)</a>"#)?;
                let re2 = Regex::new(r#"(?s)<h3[^>]*>[^<]*<a[^>]*href="(https?://[^"]+)"[^>]*>(.*?)</a>"#)?;
                let re_snip = Regex::new(r#"(?s)<p class="content"[^>]*>(.*?)</p>"#)?;
                let mut out = Vec::new();
                for caps in re.captures_iter(&resp.body).chain(re2.captures_iter(&resp.body)) {
                    let snippet = re_snip
                        .captures_iter(&resp.body)
                        .nth(out.len())
                        .map(|c| clean(&c[1]))
                        .unwrap_or_default();
                    out.push(mk("searx", &clean(&caps[2]), &caps[1], &snippet));
                    if out.len() >= limit {
                        break;
                    }
                }
                if !out.is_empty() {
                    return Ok(out);
                }
                errors.push(format!("{base}: html responded without parseable results"));
            }
            Err(e) => errors.push(format!("{base}: {e}")),
        }
    }
    Err(anyhow!(errors.join("; ")))
}

// ---------------------------------------------------------------------------
// Mojeek (independent index, no key; often blocks datacenter IPs)
// ---------------------------------------------------------------------------

pub fn mojeek(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let url = format!("https://www.mojeek.com/search?q={}", enc(q));
    let resp = fetch(http, cfg, &url, &[])?;
    let re = Regex::new(r#"(?s)<a class="ob" href="([^"]+)"[^>]*>(.*?)</a>"#)?;
    let re_snip = Regex::new(r#"(?s)<p class="s">(.*?)</p>"#)?;
    let mut out = Vec::new();
    for caps in re.captures_iter(&resp.body) {
        let snippet = re_snip
            .captures_iter(&resp.body)
            .nth(out.len())
            .map(|c| clean(&c[1]))
            .unwrap_or_default();
        out.push(mk("mojeek", &clean(&caps[2]), &caps[1], &snippet));
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!("mojeek returned no parseable results (blocked or layout change)"));
    }
    Ok(out)
}
