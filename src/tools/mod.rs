//! The complete tool surface exposed to the model.
//!
//! There is deliberately no shell, no file editor and no generic filesystem tool here:
//! the agent can only search the web, fetch URLs through the hardened curl tool, run
//! network-only Python, and manage its own visualizations. Everything else is absent
//! from the registry, so it cannot be called even by a manipulated model.

pub mod curl_tool;
pub mod html_tools;
pub mod scripts_tool;
pub mod web_search;

use crate::config::Config;
use crate::html::model::CallStep;
use crate::http::Http;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Mutable per-session state shared by tools (used for provenance in reports).
#[derive(Debug, Default, Clone)]
pub struct SessionState {
    /// Visualization names created during this session.
    pub created_reports: Vec<String>,
    pub tool_calls: u32,
    pub engines_used: BTreeSet<String>,
    pub blocked_requests: Vec<String>,
    pub fetched_urls: Vec<String>,
    /// Every tool call of this session, in order. The audit trail lives in the
    /// report, not in the terminal.
    pub call_log: Vec<CallStep>,
}

pub struct Ctx {
    pub cfg: Config,
    pub http: Http,
    pub session: SessionState,
}

impl Ctx {
    pub fn new(cfg: Config) -> Self {
        let matcher = crate::blacklist::Matcher::new(&cfg.search.blacklist);
        let http = Http::with_retries(
            matcher,
            cfg.curl.allow_private_addresses,
            cfg.net.retries,
            cfg.net.backoff_ms,
        );
        Self {
            cfg,
            http,
            session: SessionState::default(),
        }
    }
}

pub type ToolFn = fn(&mut Ctx, &Value) -> Result<Value>;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub handler: ToolFn,
}

pub fn registry() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "web_search",
            description: "Search the web through multiple independent engines (AnySearch CLI when installed, Bing, Wikipedia, DuckDuckGo, SearxNG, Google News, Crossref, OpenAlex, arXiv, Hacker News and optional key-based providers). Returns ranked results with title, url, domain, snippet, trust prior, cross-engine corroboration count and publication date. Use scope to target news / academic / code / qa sources. Set deep=true to also fetch and extract the text of the top pages so claims can be checked against primary sources; pages that a direct fetch cannot read (JS-heavy sites) are retried through the AnySearch extractor automatically. Craft each query as 2-6 precise keywords (people, organisations, product names, terms) instead of a full sentence; when freshness matters, add an explicit time qualifier such as \"February 2026\" or \"this week\".",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query: 2-6 precise keywords, not a full sentence. Add an explicit time qualifier when freshness matters, e.g. \"February 2026\" or \"this week\"." },
                    "scope": { "type": "string", "enum": ["web", "news", "academic", "code", "qa", "all"], "description": "Which engine group to use (default web)." },
                    "limit": { "type": "integer", "description": "Maximum results to return (default 12, max 40)." },
                    "engines": { "type": "array", "items": { "type": "string" }, "description": "Optional explicit engine list, e.g. [\"bing\",\"wikipedia\"]." },
                    "deep": { "type": "boolean", "description": "Also fetch the top pages and return extracted text excerpts (default false)." },
                    "deep_pages": { "type": "integer", "description": "How many top pages to fetch when deep=true (default 3, max 8)." }
                },
                "required": ["query"]
            }),
            handler: web_search::run,
        },
        ToolSpec {
            name: "curl_tool",
            description: "Fetch a single URL over HTTP/HTTPS and return its status, headers and body (HTML is converted to readable text by default). This is a hardened curl: only http/https, no file:// or local paths, no writes to disk unless save=true, domain blacklist and private-address guard enforced, response size capped. Use it to open a specific source and quote it precisely. Supports GET/HEAD/POST/PUT/PATCH/DELETE, custom headers, form or JSON bodies. Set render=true to have the AnySearch extractor render a JS-heavy page into Markdown (slower, and the URL is shared with that third-party service).",
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Absolute http(s) URL." },
                    "method": { "type": "string", "enum": ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"], "description": "HTTP method (default GET)." },
                    "headers": { "type": "object", "description": "Extra request headers as an object, e.g. {\"Accept\":\"application/json\"}." },
                    "body": { "type": "string", "description": "Raw request body for POST/PUT/PATCH." },
                    "json": { "type": "object", "description": "JSON body (sets Content-Type: application/json)." },
                    "extract_text": { "type": "boolean", "description": "Convert HTML to readable text (default true)." },
                    "include_headers": { "type": "boolean", "description": "Include response headers in the result (default false)." },
                    "save": { "type": "boolean", "description": "Save the raw body under the workspace raw/ directory and return the path (default false)." },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default from config, max 120)." },
                    "max_bytes": { "type": "integer", "description": "Maximum body bytes to read (default from config)." },
                    "follow_redirects": { "type": "boolean", "description": "Follow redirects (default true)." },
                    "render": { "type": "boolean", "description": "Render the page with the AnySearch extractor and return Markdown (default false). Use it when a page is empty or JS-driven. Same domain/private-address policy applies." }
                },
                "required": ["url"]
            }),
            handler: curl_tool::run,
        },
        ToolSpec {
            name: "scripts_tool",
            description: "Run a short Python script in a network-only sandbox to parse, cross-check or aggregate data you already fetched. Only network/parsing standard-library modules are importable; filesystem, process, socket and dynamic-execution APIs are rejected before the script runs. A prelude provides fetch(url, ...) and fetch_json(url, ...) which enforce the same domain policy as curl_tool. Use it for regex extraction, JSON reshaping, date diffing, statistics and cross-source dedupe.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "Python 3 source. Print a JSON result to stdout with print(json.dumps(...))." },
                    "timeout_secs": { "type": "integer", "description": "Wall-clock limit (default from config, max 120)." },
                    "stdin": { "type": "string", "description": "Optional text piped to the script's stdin." }
                },
                "required": ["code"]
            }),
            handler: scripts_tool::run,
        },
        ToolSpec {
            name: "create_html",
            description: "Create a single-file offline HTML visualization that summarizes the verified findings: info cards per claim with verification status and confidence, a source table where every source is a real hyperlink with reliability scoring, a timeline, and an abstract event-correlation graph (force-directed, rendered from the relations you supply). Returns the file path. Never invent URLs: only pass URLs that tools actually returned.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short kebab-case id used for the file name, e.g. \"rust-adoption-2026\"." },
                    "title": { "type": "string", "description": "Human-readable report title." },
                    "query": { "type": "string", "description": "The original question this report answers." },
                    "summary": { "type": "string", "description": "Markdown-ish executive summary (a few short paragraphs)." },
                    "conclusion": { "type": "string", "description": "The closing answer shown in the report's conclusion section; optional, since it is normally injected automatically when the run ends." },
                    "items": {
                        "type": "array",
                        "description": "Claims / findings.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "claim": { "type": "string" },
                                "detail": { "type": "string" },
                                "status": { "type": "string", "enum": ["verified", "supported", "unverified", "disputed", "refuted"] },
                                "confidence": { "type": "integer", "description": "0-100" },
                                "source_ids": { "type": "array", "items": { "type": "string" }, "description": "Ids of the sources supporting this claim." },
                                "tags": { "type": "array", "items": { "type": "string" } },
                                "date": { "type": "string", "description": "ISO date of the underlying event, if any." }
                            },
                            "required": ["claim"]
                        }
                    },
                    "sources": {
                        "type": "array",
                        "description": "Every source that was actually retrieved.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "title": { "type": "string" },
                                "url": { "type": "string" },
                                "kind": { "type": "string", "enum": ["primary", "secondary", "academic", "official", "social", "unknown"] },
                                "reliability": { "type": "integer", "description": "0-100" },
                                "published": { "type": "string" },
                                "quote": { "type": "string", "description": "Exact quoted passage used as evidence." },
                                "engine": { "type": "string" }
                            },
                            "required": ["url"]
                        }
                    },
                    "events": {
                        "type": "array",
                        "description": "Timeline entries or abstract nodes of the correlation graph.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "label": { "type": "string" },
                                "date": { "type": "string" },
                                "description": { "type": "string" },
                                "kind": { "type": "string", "description": "event | claim | actor | cause | effect" },
                                "source_ids": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["label"]
                        }
                    },
                    "relations": {
                        "type": "array",
                        "description": "Edges of the correlation graph.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string", "description": "Event id or label." },
                                "to": { "type": "string" },
                                "kind": { "type": "string", "enum": ["causes", "supports", "contradicts", "follows", "related", "mentions"] },
                                "note": { "type": "string" },
                                "strength": { "type": "integer", "description": "1-5" }
                            },
                            "required": ["from", "to"]
                        }
                    },
                    "notes": { "type": "array", "items": { "type": "string" }, "description": "Caveats, open questions, methodology notes." },
                    "theme": { "type": "string", "enum": ["dark", "light"], "description": "Visual theme (default from config)." }
                },
                "required": ["name", "title"]
            }),
            handler: html_tools::create,
        },
        ToolSpec {
            name: "edit_html",
            description: "Modify an existing visualization created by create_html. Reports are stored as a JSON model plus a rendered HTML file, so edits are applied to the model and the HTML is re-rendered consistently. Supports adding/updating/removing items, sources, events, relations and notes, and changing the title/summary/theme.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Report id (file stem) created earlier." },
                    "set_title": { "type": "string" },
                    "set_summary": { "type": "string" },
                    "set_conclusion": { "type": "string", "description": "The closing answer shown in the report's conclusion section (usually injected automatically when a run ends)." },
                    "set_theme": { "type": "string", "enum": ["dark", "light"] },
                    "add_items": { "type": "array", "items": { "type": "object" } },
                    "update_items": { "type": "array", "items": { "type": "object" }, "description": "Objects with id plus fields to change." },
                    "remove_item_ids": { "type": "array", "items": { "type": "string" } },
                    "add_sources": { "type": "array", "items": { "type": "object" } },
                    "update_sources": { "type": "array", "items": { "type": "object" } },
                    "remove_source_ids": { "type": "array", "items": { "type": "string" } },
                    "add_events": { "type": "array", "items": { "type": "object" } },
                    "remove_event_ids": { "type": "array", "items": { "type": "string" } },
                    "add_relations": { "type": "array", "items": { "type": "object" } },
                    "remove_relations": { "type": "array", "items": { "type": "object" }, "description": "{\"from\":..,\"to\":..} pairs to drop." },
                    "add_notes": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["name"]
            }),
            handler: html_tools::edit,
        },
        ToolSpec {
            name: "delete_html",
            description: "Delete a visualization (HTML plus its JSON model) that this agent produced. Refuses paths outside the visualization directory. Use it to clean up superseded or mistaken reports.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Report id to delete. Use \"*\" with confirm_all=true to delete every report." },
                    "confirm_all": { "type": "boolean", "description": "Required when name is \"*\"." }
                },
                "required": ["name"]
            }),
            handler: html_tools::delete,
        },
    ]
}

pub fn dispatch(ctx: &mut Ctx, name: &str, args: &Value) -> Result<Value> {
    let specs = registry();
    let spec = specs
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow!("unknown tool '{name}'. Available tools: {}", specs.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")))?;
    ctx.session.tool_calls += 1;
    (spec.handler)(ctx, args)
}

/// The three tools that only exist to produce the HTML report.
pub const REPORT_TOOLS: [&str; 3] = ["create_html", "edit_html", "delete_html"];

/// The tool names offered to the model for this config. Without `viz.auto_html` the
/// three report tools are dropped from the surface entirely — capability, not a hint
/// in the prompt, is what keeps the default run report-free.
pub fn surface(cfg: &Config) -> Vec<&'static str> {
    let all: Vec<&'static str> = registry().iter().map(|t| t.name).collect();
    if cfg.viz.auto_html {
        all
    } else {
        all.into_iter().filter(|n| !REPORT_TOOLS.contains(n)).collect()
    }
}

/// OpenAI / DeepSeek compatible tool declaration payload for the tools a
/// configuration enables.
pub fn openai_tools_for_config(cfg: &Config) -> Value {
    openai_tools_for(&surface(cfg))
}

/// The same payload restricted to a subset of the registry, used for the
/// create_html-only catch-up round at the end of a run.
pub fn openai_tools_for(names: &[&str]) -> Value {
    Value::Array(
        registry()
            .iter()
            .filter(|t| names.contains(&t.name))
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect(),
    )
}

/// Same list, restricted to the tools a given config / surface enables.
pub fn describe_for(names: &[&'static str]) -> Vec<(&'static str, &'static str, Vec<String>)> {
    registry()
        .iter()
        .filter(|t| names.contains(&t.name))
        .map(|t| {
            let required: Vec<String> = t
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            (t.name, t.description, required)
        })
        .collect()
}

/// Argument accessor with type coercion and clear errors.
pub struct Args<'a> {
    v: &'a Value,
}

impl<'a> Args<'a> {
    pub fn new(v: &'a Value) -> Self {
        Self { v }
    }

    pub fn req_str(&self, key: &str) -> Result<String> {
        self.str(key)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow!("missing required parameter '{key}'"))
    }

    pub fn str(&self, key: &str) -> Option<String> {
        match self.v.get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            Some(Value::Bool(b)) => Some(b.to_string()),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        match self.v.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
            _ => default,
        }
    }

    pub fn u64(&self, key: &str, default: u64) -> u64 {
        match self.v.get(key) {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(default),
            Some(Value::String(s)) => s.parse().unwrap_or(default),
            _ => default,
        }
    }

    pub fn array(&self, key: &str) -> Vec<Value> {
        match self.v.get(key) {
            Some(Value::Array(a)) => a.clone(),
            Some(other) if !other.is_null() => vec![other.clone()],
            _ => Vec::new(),
        }
    }

    pub fn strings(&self, key: &str) -> Vec<String> {
        self.array(key)
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    }

    pub fn object(&self, key: &str) -> Vec<(String, String)> {
        match self.v.get(key) {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(k, v)| {
                    let val = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), val)
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_exactly_the_sealed_surface() {
        let names: Vec<&str> = registry().iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["web_search", "curl_tool", "scripts_tool", "create_html", "edit_html", "delete_html"]
        );
        // no shell / editor / generic filesystem tools may ever appear
        for banned in ["bash", "shell", "exec", "edit", "write", "read", "glob", "grep"] {
            assert!(!names.contains(&banned), "banned tool name leaked into the registry: {banned}");
        }
    }

    #[test]
    fn the_report_tools_are_opt_in() {
        // The registry holds all six; the *surface* is what the model sees, and by
        // default that is the three research tools only.
        let mut cfg = Config::default();
        assert_eq!(surface(&cfg), vec!["web_search", "curl_tool", "scripts_tool"]);
        assert_eq!(openai_tools_for_config(&cfg).as_array().unwrap().len(), 3);
        assert_eq!(describe_for(&surface(&cfg)).len(), 3);

        cfg.viz.auto_html = true;
        assert_eq!(surface(&cfg).len(), 6);
        assert_eq!(openai_tools_for_config(&cfg).as_array().unwrap().len(), 6);
        assert_eq!(describe_for(&surface(&cfg)).len(), 6);
    }

    #[test]
    fn every_tool_has_a_valid_schema() {
        for spec in registry() {
            assert_eq!(spec.parameters.get("type").and_then(|t| t.as_str()), Some("object"));
            assert!(spec.parameters.get("properties").is_some(), "{} has no properties", spec.name);
            assert!(spec.description.len() > 40, "{} description too thin", spec.name);
        }
        let payload = openai_tools_for_config(&Config::default());
        assert_eq!(payload.as_array().unwrap().len(), 3, "default surface is research-only");
        let mut report_cfg = Config::default();
        report_cfg.viz.auto_html = true;
        assert_eq!(
            openai_tools_for_config(&report_cfg).as_array().unwrap().len(),
            6,
            "the report tools come back with viz.auto_html"
        );
    }

    #[test]
    fn tool_subset_payload_is_faithful() {
        let one = openai_tools_for(&["create_html"]);
        let arr = one.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["function"]["name"], json!("create_html"));
        assert_eq!(arr[0]["function"]["parameters"]["type"], json!("object"));
    }

    #[test]
    fn args_coercion() {
        let v = json!({"a": "x", "b": true, "c": 5, "d": ["p","q"], "e": {"H":"v"}});
        let a = Args::new(&v);
        assert_eq!(a.req_str("a").unwrap(), "x");
        assert!(a.bool("b", false));
        assert_eq!(a.u64("c", 1), 5);
        assert_eq!(a.strings("d"), vec!["p", "q"]);
        assert_eq!(a.object("e"), vec![("H".to_string(), "v".to_string())]);
        assert!(a.req_str("zzz").is_err());
    }

    #[test]
    fn unknown_tool_is_rejected() {
        let mut ctx = Ctx::new(Config::default());
        let err = dispatch(&mut ctx, "bash", &json!({"command": "rm -rf /"})).unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }
}
