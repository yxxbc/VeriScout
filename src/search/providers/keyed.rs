//! Optional key-based engines. Each one is skipped unless the matching key is configured,
//! so the default install stays fully key-free.

use super::{clean, enc, fetch, json_at, json_str_at, mk, post_json};
use crate::config::Config;
use crate::http::Http;
use crate::search::SearchResult;
use anyhow::{anyhow, Result};
use serde_json::json;

pub fn tavily(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let key = cfg
        .search_api_key(&cfg.search.tavily_api_key_env)
        .ok_or_else(|| anyhow!("tavily key not configured (env {})", cfg.search.tavily_api_key_env))?;
    let body = json!({
        "api_key": key,
        "query": q,
        "max_results": limit,
        "search_depth": "advanced",
        "include_answer": false,
    });
    let resp = post_json(http, cfg, "https://api.tavily.com/search", &[], &body)?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let items = json_at(&v, &["results"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("tavily response missing results"))?;
    let mut out = Vec::new();
    for item in items {
        let title = json_str_at(item, &["title"]).unwrap_or_default();
        let url = json_str_at(item, &["url"]).unwrap_or_default();
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let content = json_str_at(item, &["content"]).unwrap_or_default();
        let mut r = mk("tavily", &title, &url, &content);
        r.published = json_str_at(item, &["published_date"]);
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("tavily returned no results"));
    }
    Ok(out)
}

pub fn serper(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let key = cfg
        .search_api_key(&cfg.search.serper_api_key_env)
        .ok_or_else(|| anyhow!("serper key not configured (env {})", cfg.search.serper_api_key_env))?;
    let body = json!({ "q": q, "num": limit, "gl": "us", "hl": "en" });
    let resp = post_json(
        http,
        cfg,
        "https://google.serper.dev/search",
        &[("X-API-KEY", key.as_str())],
        &body,
    )?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let mut out = Vec::new();
    if let Some(items) = v.get("organic").and_then(|x| x.as_array()) {
        for item in items {
            let title = json_str_at(item, &["title"]).unwrap_or_default();
            let url = json_str_at(item, &["link"]).unwrap_or_default();
            if title.is_empty() || !url.starts_with("http") {
                continue;
            }
            let snippet = json_str_at(item, &["snippet"]).unwrap_or_default();
            let mut r = mk("serper", &title, &url, &snippet);
            r.published = json_str_at(item, &["date"]);
            out.push(r);
        }
    }
    if out.is_empty() {
        return Err(anyhow!("serper returned no organic results"));
    }
    Ok(out)
}

pub fn brave(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let key = cfg
        .search_api_key(&cfg.search.brave_api_key_env)
        .ok_or_else(|| anyhow!("brave key not configured (env {})", cfg.search.brave_api_key_env))?;
    let url = format!("https://api.search.brave.com/res/v1/web/search?q={}&count={}", enc(q), limit);
    let resp = fetch(
        http,
        cfg,
        &url,
        &[
            ("X-Subscription-Token", key.as_str()),
            ("Accept", "application/json"),
        ],
    )?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)?;
    let items = json_at(&v, &["web", "results"])
        .and_then(|x| x.as_array())
        .ok_or_else(|| anyhow!("brave response missing web.results"))?;
    let mut out = Vec::new();
    for item in items {
        let title = json_str_at(item, &["title"]).unwrap_or_default();
        let url = json_str_at(item, &["url"]).unwrap_or_default();
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let desc = json_str_at(item, &["description"]).unwrap_or_default();
        let mut r = mk("brave", &title, &url, &desc);
        r.published = json_str_at(item, &["age"]);
        out.push(r);
    }
    if out.is_empty() {
        return Err(anyhow!("brave returned no results"));
    }
    Ok(out)
}

/// A flexible adapter for self-hosted / third-party aggregators (e.g. an "anysearch"
/// bridge). Point `search.custom_endpoint` at any JSON API that accepts
/// `{"query": "...", "limit": N}` and returns a list of `{title,url,snippet}`-shaped objects.
pub fn custom(http: &Http, cfg: &Config, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let endpoint = cfg.search.custom_endpoint.trim();
    if endpoint.is_empty() {
        return Err(anyhow!("search.custom_endpoint is not configured"));
    }
    let key = cfg.search_api_key(&cfg.search.custom_api_key_env).unwrap_or_default();
    let body = json!({ "query": q, "q": q, "limit": limit });
    let auth = if key.is_empty() { String::new() } else { format!("Bearer {key}") };
    let headers: Vec<(&str, &str)> = if auth.is_empty() {
        vec![]
    } else {
        vec![("Authorization", auth.as_str())]
    };
    let resp = post_json(http, cfg, endpoint, &headers, &body)?;
    let v: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| anyhow!("custom endpoint did not return JSON: {e}"))?;
    let arrays: Vec<&Vec<serde_json::Value>> = ["results", "data", "items", "answer", "documents"]
        .iter()
        .filter_map(|k| v.get(*k).and_then(|x| x.as_array()))
        .collect();
    let items: Vec<&serde_json::Value> = if let Some(arr) = arrays.first() {
        arr.iter().collect()
    } else if let Some(arr) = v.as_array() {
        arr.iter().collect()
    } else {
        return Err(anyhow!("custom endpoint JSON has no recognisable result array"));
    };
    let pick = |item: &serde_json::Value, keys: &[&str]| -> String {
        for k in keys {
            if let Some(s) = json_str_at(item, &[k]) {
                return s;
            }
        }
        String::new()
    };
    let mut out = Vec::new();
    for item in items {
        let title = pick(item, &["title", "name", "headline", "label"]);
        let url = pick(item, &["url", "link", "href", "source_url"]);
        if title.is_empty() || !url.starts_with("http") {
            continue;
        }
        let snippet = clean(&pick(item, &["snippet", "content", "description", "summary", "text", "body"]));
        let mut r = mk("custom", &title, &url, &snippet);
        r.published = Some(pick(item, &["published", "date", "published_date", "timestamp"]))
            .filter(|s| !s.is_empty());
        out.push(r);
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!("custom endpoint returned no usable results"));
    }
    Ok(out)
}
