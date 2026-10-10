//! Individual search engine adapters.
//!
//! Every adapter returns `SearchResult` rows and must be resilient: a blocked or
//! changed endpoint yields an `Err` (surfaced in the report) rather than a panic.

pub mod anysearch;
pub mod general;
pub mod keyed;
pub mod vertical;

use super::SearchResult;
use crate::blacklist::trust_prior;
use crate::config::Config;
use crate::http::Http;
use crate::util;
use anyhow::{anyhow, Result};

pub fn run(engine: &str, http: &Http, cfg: &Config, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    match engine {
        "anysearch" | "any" => anysearch::search(cfg, query, limit),
        "bing" => general::bing(http, cfg, query, limit),
        "wikipedia" => general::wikipedia(http, cfg, query, limit),
        "duckduckgo" | "ddg" => general::duckduckgo(http, cfg, query, limit),
        "searx" | "searxng" => general::searx(http, cfg, query, limit),
        "mojeek" => general::mojeek(http, cfg, query, limit),
        "googlenews" | "news" => vertical::google_news(http, cfg, query, limit),
        "crossref" => vertical::crossref(http, cfg, query, limit),
        "openalex" => vertical::openalex(http, cfg, query, limit),
        "arxiv" => vertical::arxiv(http, cfg, query, limit),
        "hackernews" | "hn" => vertical::hackernews(http, cfg, query, limit),
        "github" => vertical::github(http, cfg, query, limit),
        "stackexchange" | "stackoverflow" => vertical::stack_exchange(http, cfg, query, limit),
        "tavily" => keyed::tavily(http, cfg, query, limit),
        "serper" => keyed::serper(http, cfg, query, limit),
        "brave" => keyed::brave(http, cfg, query, limit),
        "custom" => keyed::custom(http, cfg, query, limit),
        other => Err(anyhow!("unknown search engine '{other}'")),
    }
}

/// Build a result row, filling in domain and trust prior consistently.
pub fn mk(engine: &str, title: &str, url: &str, snippet: &str) -> SearchResult {
    let domain = util::host_of(url).unwrap_or_default();
    SearchResult {
        title: clean(title),
        url: url.trim().to_string(),
        snippet: clean(snippet),
        engine: engine.to_string(),
        trust: trust_prior(&domain),
        domain,
        score: 0.0,
        corroboration: 1,
        published: None,
        extra: None,
    }
}

/// Strip markup/entities and collapse whitespace.
pub fn clean(s: &str) -> String {
    let no_tags = util::strip_tags(s);
    let decoded = util::decode_entities(&no_tags);
    util::collapse_whitespace(&decoded)
}

/// Percent-encode a query string for use as a URL query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

/// Percent-encode a URL path segment (used for Wikipedia article titles).
pub fn enc_path(s: &str) -> String {
    enc(&s.replace(' ', "_")).replace("%2F", "/")
}

/// Text between two literal markers.
pub fn between<'a>(hay: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = hay.find(start)? + start.len();
    let j = hay[i..].find(end)? + i;
    Some(&hay[i..j])
}

/// Fetch a URL with the search configuration applied.
pub fn fetch(http: &Http, cfg: &Config, url: &str, extra_headers: &[(&str, &str)]) -> Result<crate::http::Response> {
    let mut req = crate::http::Request::get(url)
        .timeout(cfg.search.timeout_secs)
        .max_bytes(cfg.search.max_body_bytes)
        .ua(&cfg.search.user_agent);
    for (k, v) in extra_headers {
        req = req.header(k, v);
    }
    http.fetch(&req)
}

/// POST JSON helper shared by the key-based adapters.
pub fn post_json(http: &Http, cfg: &Config, url: &str, headers: &[(&str, &str)], body: &serde_json::Value) -> Result<crate::http::Response> {
    let mut req = crate::http::Request::get(url)
        .method("POST")
        .header("Content-Type", "application/json")
        .timeout(cfg.search.timeout_secs)
        .max_bytes(cfg.search.max_body_bytes)
        .ua(&cfg.search.user_agent)
        .body(serde_json::to_vec(body)?);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    http.fetch(&req)
}

/// First JSON string found at the given key path, e.g. ["items", "0", "title"].
pub fn json_at<'a>(v: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cur = v;
    for key in path {
        cur = match cur {
            serde_json::Value::Object(map) => map.get(*key)?,
            serde_json::Value::Array(arr) => arr.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

pub fn json_str_at(v: &serde_json::Value, path: &[&str]) -> Option<String> {
    match json_at(v, path)? {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).map(|s| s.to_string()),
        _ => None,
    }
}

