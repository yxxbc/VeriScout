//! web_search - multi-engine aggregation with blacklist filtering and optional deep fetch.

use super::{Args, Ctx};
use crate::http::Request;
use crate::search::{Aggregator, Scope};
use crate::util;
use anyhow::Result;
use serde_json::{json, Value};

pub fn run(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = Args::new(args);
    let query = a.req_str("query")?;
    let scope = Scope::parse(&a.str("scope").unwrap_or_default());
    let limit = a.u64("limit", 12).clamp(1, 40) as usize;
    let engines = {
        let list = a.strings("engines");
        if list.is_empty() { None } else { Some(list) }
    };
    let deep = a.bool("deep", false);
    let deep_pages = a.u64("deep_pages", 3).clamp(0, 8) as usize;

    let aggregator = Aggregator::new(&ctx.cfg, &ctx.http);
    let report = aggregator.search(&query, scope, limit, engines.as_deref());

    for status in &report.engines {
        if status.ok {
            ctx.session.engines_used.insert(status.engine.clone());
        }
    }

    let mut results: Vec<Value> = Vec::new();
    for (idx, r) in report.results.iter().enumerate() {
        results.push(json!({
            "n": idx + 1,
            "title": r.title,
            "url": r.url,
            "domain": r.domain,
            "snippet": r.snippet,
            "engines": r.engine,
            "trust": r.trust,
            "score": r.score,
            "corroboration": r.corroboration,
            "published": r.published,
            "extra": r.extra,
        }));
    }

    let mut pages: Vec<Value> = Vec::new();
    if deep && deep_pages > 0 {
        for r in report.results.iter().take(deep_pages) {
            pages.push(fetch_page(ctx, &r.url, &r.title));
        }
    }

    Ok(json!({
        "query": report.query,
        "scope": report.scope,
        "took_ms": report.took_ms,
        "count": results.len(),
        "results": results,
        "engines": report.engines.iter().map(|e| json!({
            "engine": e.engine, "ok": e.ok, "results": e.results, "ms": e.ms, "error": e.error
        })).collect::<Vec<_>>(),
        "blacklisted_filtered": report.filtered,
        "notes": report.notes,
        "pages": pages,
        "guidance": "Cite only URLs from this list. A claim backed by a single source is 'unverified'/'supported' at best; two independent domains make it 'verified'.",
    }))
}

/// Fetch one result page. When a direct fetch fails, or returns almost no readable
/// text (JS-rendered sites), fall back to the AnySearch extractor - but only for URLs
/// that already passed the same protocol/host policy, so a blocked target is never
/// handed to a third party either.
fn fetch_page(ctx: &Ctx, url: &str, title: &str) -> Value {
    let req = Request::get(url)
        .timeout(ctx.cfg.search.timeout_secs)
        .max_bytes(ctx.cfg.search.max_body_bytes)
        .ua(&ctx.cfg.search.user_agent);
    let (text, status, bytes, content_type, truncated, error) = match ctx.http.fetch(&req) {
        Ok(resp) => {
            let text = resp.text();
            (text, resp.status, resp.bytes, resp.content_type, resp.truncated, None)
        }
        Err(e) => (String::new(), 0u16, 0usize, String::new(), false, Some(e.to_string())),
    };

    let thin = text.trim().chars().count() < 200;
    if (thin || error.is_some()) && ctx.cfg.search.anysearch_extract && ctx.http.vet_url(url).is_ok() {
        match crate::search::providers::anysearch::extract(&ctx.cfg, url) {
            Ok(markdown) => {
                let (excerpt, cut) = util::truncate_chars(markdown.trim(), 8_000);
                let mut payload = json!({
                    "url": url,
                    "title": title,
                    "status": 200,
                    "extractor": "anysearch",
                    "content_type": "text/markdown",
                    "excerpt": excerpt,
                    "truncated": cut,
                    "note": "readable text obtained through the AnySearch extractor because the direct fetch was empty or too thin",
                });
                if let Some(err) = &error {
                    payload["http_error"] = json!(err);
                }
                return payload;
            }
            Err(e) => {
                if error.is_some() {
                    return json!({ "url": url, "title": title, "error": error, "extractor_error": e.to_string() });
                }
            }
        }
    }

    if let Some(err) = error {
        return json!({ "url": url, "title": title, "error": err });
    }
    let (excerpt, cut) = util::truncate_chars(text.trim(), 6_000);
    json!({
        "url": url,
        "title": title,
        "status": status,
        "bytes": bytes,
        "content_type": content_type,
        "extractor": "http",
        "excerpt": excerpt,
        "truncated": truncated || cut,
    })
}
