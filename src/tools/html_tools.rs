//! create_html / edit_html / delete_html - the only file-producing tools in the surface.
//!
//! Reports live in `<workspace>/<viz.dir>/<name>.html` with a sibling `<name>.json`
//! model. Edits always go through the model and re-render, so the HTML can never drift
//! away from the data. Paths are slug-validated, so no argument can escape the
//! visualization directory.

use super::{Args, Ctx};
use crate::html::model::*;
use crate::html::render;
use crate::util;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn safe_name(raw: &str) -> Result<String> {
    let name = util::slugify(raw);
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.starts_with('.') {
        return Err(anyhow!("invalid report name '{raw}'"));
    }
    if name.len() > 70 {
        return Err(anyhow!("report name too long (max 70 chars after slugging)"));
    }
    Ok(name)
}

fn paths(ctx: &Ctx, name: &str) -> (PathBuf, PathBuf) {
    let dir = ctx.cfg.viz_dir();
    (dir.join(format!("{name}.html")), dir.join(format!("{name}.json")))
}

/// Refuse to write anywhere that is not inside the visualization directory.
fn assert_inside(dir: &Path, path: &Path) -> Result<()> {
    let dir_canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let parent = path.parent().unwrap_or(dir);
    let parent_canon = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
    if !parent_canon.starts_with(&dir_canon) {
        return Err(anyhow!(
            "refusing to touch {}: outside the visualization directory {}",
            path.display(),
            dir_canon.display()
        ));
    }
    Ok(())
}

/// Parse a list tolerantly: one malformed element must never discard the whole list.
///
/// Accepts an array, a single object, or an object-map (`{"1": {...}}`), and reports
/// each skipped element instead of failing silently.
fn parse_list<T: serde::de::DeserializeOwned>(
    raw: Option<&Value>,
    field: &str,
    warnings: &mut Vec<String>,
    validate: fn(&T) -> Option<String>,
) -> Vec<T> {
    let elements: Vec<Value> = match raw {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(m)) => m.values().cloned().collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    };
    let mut out = Vec::new();
    for (i, elem) in elements.iter().enumerate() {
        match serde_json::from_value::<T>(elem.clone()) {
            Ok(v) => match validate(&v) {
                Some(reason) => warnings.push(format!("{field}[{i}] was skipped: {reason}")),
                None => out.push(v),
            },
            Err(e) => warnings.push(format!("{field}[{i}] was skipped: {e}")),
        }
    }
    out
}

fn require_claim(item: &Item) -> Option<String> {
    if item.claim.trim().is_empty() {
        Some("empty claim".into())
    } else {
        None
    }
}

fn require_label(event: &Event) -> Option<String> {
    if event.label.trim().is_empty() {
        Some("empty label".into())
    } else {
        None
    }
}

fn require_endpoints(relation: &Relation) -> Option<String> {
    if relation.from.trim().is_empty() || relation.to.trim().is_empty() {
        Some("missing from/to".into())
    } else {
        None
    }
}

fn accept_any<T>(_: &T) -> Option<String> {
    None
}

/// Collect the URLs the model is allowed to cite: http(s) only.
fn vet_sources(report: &mut Report) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut kept = Vec::new();
    for s in report.sources.drain(..) {
        let url = s.url.trim().to_string();
        if url.is_empty() {
            warnings.push(format!("dropped a source without a URL (title: {})", s.title));
            continue;
        }
        match url::Url::parse(&url) {
            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => kept.push(Source { url, ..s }),
            _ => warnings.push(format!("dropped non-http(s) source URL: {url}")),
        }
    }
    report.sources = kept;
    warnings
}

pub fn create(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = Args::new(args);
    let raw_name = a.req_str("name")?;
    let name = safe_name(&raw_name)?;
    let title = a.req_str("title")?;

    let dir = ctx.cfg.viz_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;

    // never silently clobber an existing report
    let mut final_name = name.clone();
    let mut suffix = 2;
    while paths(ctx, &final_name).0.exists() {
        final_name = format!("{name}-{suffix}");
        suffix += 1;
        if suffix > 50 {
            return Err(anyhow!("too many reports named like '{name}'; delete some first"));
        }
    }

    let mut report = Report {
        name: final_name.clone(),
        title,
        query: a.str("query").unwrap_or_default(),
        theme: a.str("theme").unwrap_or_else(|| ctx.cfg.viz.theme.clone()),
        locale: ctx.cfg.viz.locale.clone(),
        summary: a.str("summary").unwrap_or_default(),
        conclusion: a.str("conclusion").unwrap_or_default(),
        ..Default::default()
    };
    let mut warnings: Vec<String> = Vec::new();
    report.items = parse_list(args.get("items"), "items", &mut warnings, require_claim);
    report.sources = parse_list(args.get("sources"), "sources", &mut warnings, accept_any::<Source>);
    report.events = parse_list(args.get("events"), "events", &mut warnings, require_label);
    report.relations = parse_list(args.get("relations"), "relations", &mut warnings, require_endpoints);
    report.notes = a
        .strings("notes")
        .into_iter()
        .map(|text| Note { at: String::new(), text })
        .collect();
    let supplied_items = args.get("items").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    if report.items.is_empty() && supplied_items == 0 {
        warnings.push("no claims (items) were supplied - the findings section will be empty".into());
    }
    warnings.extend(vet_sources(&mut report));
    report.provenance = provenance_snapshot(ctx);
    report.normalize();

    if report.sources.is_empty() {
        warnings.push("no sources were attached: a report without sources is an unverified draft".into());
    }
    if report.items.iter().any(|i| i.source_ids.is_empty()) {
        warnings.push("some claims have no source attached".into());
    }

    let metrics = report.metrics();
    let html = render::render(&report);
    let (html_path, json_path) = paths(ctx, &final_name);
    assert_inside(&dir, &html_path)?;
    assert_inside(&dir, &json_path)?;
    std::fs::write(&html_path, &html)?;
    if ctx.cfg.viz.keep_model {
        std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    }
    ctx.session.created_reports.push(final_name.clone());

    Ok(json!({
        "ok": true,
        "name": final_name,
        "html_path": html_path.display().to_string(),
        "model_path": if ctx.cfg.viz.keep_model { json_path.display().to_string() } else { String::new() },
        "bytes": html.len(),
        "stats": {
            "sources": metrics.sources,
            "items": metrics.items,
            "events": metrics.events,
            "relations": metrics.relations,
            "avg_reliability": (metrics.avg_reliability * 10.0).round() / 10.0,
            "verified_ratio": (metrics.verified_ratio * 100.0).round() / 100.0,
        },
        "warnings": warnings,
        "next": "open the file in a browser; use edit_html to add findings or sources",
    }))
}

pub fn edit(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = Args::new(args);
    let name = safe_name(&a.req_str("name")?)?;
    let (html_path, json_path) = paths(ctx, &name);
    if !json_path.exists() {
        return Err(anyhow!(
            "report '{name}' has no model file at {}. Create it with create_html first.",
            json_path.display()
        ));
    }
    let raw = std::fs::read_to_string(&json_path)?;
    let mut report: Report = serde_json::from_str(&raw)
        .with_context(|| format!("model file {} is not valid JSON", json_path.display()))?;

    let mut changes: Vec<String> = Vec::new();

    if let Some(t) = a.str("set_title") {
        report.title = t;
        changes.push("title".into());
    }
    if let Some(s) = a.str("set_summary") {
        report.summary = s;
        changes.push("summary".into());
    }
    if let Some(c) = a.str("set_conclusion") {
        report.conclusion = c;
        changes.push("conclusion".into());
    }
    if let Some(theme) = a.str("set_theme") {
        report.theme = if theme.eq_ignore_ascii_case("light") { "light".into() } else { "dark".into() };
        changes.push("theme".into());
    }

    let mut warnings: Vec<String> = Vec::new();

    let new_sources: Vec<Source> = parse_list(args.get("add_sources"), "add_sources", &mut warnings, accept_any::<Source>);
    if !new_sources.is_empty() {
        changes.push(format!("add_sources({})", new_sources.len()));
    }
    report.sources.extend(new_sources);

    let new_items: Vec<Item> = parse_list(args.get("add_items"), "add_items", &mut warnings, require_claim);
    if !new_items.is_empty() {
        changes.push(format!("add_items({})", new_items.len()));
    }
    report.items.extend(new_items);

    let new_events: Vec<Event> = parse_list(args.get("add_events"), "add_events", &mut warnings, require_label);
    if !new_events.is_empty() {
        changes.push(format!("add_events({})", new_events.len()));
    }
    report.events.extend(new_events);

    let new_relations: Vec<Relation> = parse_list(args.get("add_relations"), "add_relations", &mut warnings, require_endpoints);
    if !new_relations.is_empty() {
        changes.push(format!("add_relations({})", new_relations.len()));
    }
    report.relations.extend(new_relations);

    for patch in parse_list::<Source>(args.get("update_sources"), "update_sources", &mut warnings, accept_any::<Source>) {
        if let Some(target) = report.sources.iter_mut().find(|s| s.id == patch.id || (!patch.url.is_empty() && s.url == patch.url)) {
            merge_source(target, &patch);
            changes.push(format!("update_source({})", patch.id));
        }
    }
    for patch in parse_list::<Item>(args.get("update_items"), "update_items", &mut warnings, accept_any::<Item>) {
        if let Some(target) = report.items.iter_mut().find(|i| i.id == patch.id) {
            merge_item(target, &patch);
            changes.push(format!("update_item({})", patch.id));
        }
    }

    let remove_items = a.strings("remove_item_ids");
    if !remove_items.is_empty() {
        let before = report.items.len();
        report.items.retain(|i| !remove_items.contains(&i.id));
        changes.push(format!("remove_items({})", before - report.items.len()));
    }
    let remove_sources = a.strings("remove_source_ids");
    if !remove_sources.is_empty() {
        let before = report.sources.len();
        report.sources.retain(|s| !remove_sources.contains(&s.id));
        for item in report.items.iter_mut() {
            item.source_ids.retain(|id| !remove_sources.contains(id));
        }
        changes.push(format!("remove_sources({})", before - report.sources.len()));
    }
    let remove_events = a.strings("remove_event_ids");
    if !remove_events.is_empty() {
        let before = report.events.len();
        report.events.retain(|e| !remove_events.contains(&e.id));
        changes.push(format!("remove_events({})", before - report.events.len()));
    }
    let remove_relations = a.array("remove_relations");
    if !remove_relations.is_empty() {
        let before = report.relations.len();
        report.relations.retain(|rel| {
            !remove_relations.iter().any(|target| {
                let f = target.get("from").and_then(|x| x.as_str()).unwrap_or("");
                let t = target.get("to").and_then(|x| x.as_str()).unwrap_or("");
                let rel_from = rel.from.trim().to_ascii_lowercase();
                let rel_to = rel.to.trim().to_ascii_lowercase();
                (f.is_empty() || f.trim().to_ascii_lowercase() == rel_from)
                    && (t.is_empty() || t.trim().to_ascii_lowercase() == rel_to)
            })
        });
        changes.push(format!("remove_relations({})", before - report.relations.len()));
    }
    for text in a.strings("add_notes") {
        report.notes.push(Note { at: util::iso8601_now(), text });
        changes.push("add_note".into());
    }

    if changes.is_empty() {
        return Ok(json!({ "ok": false, "error": "no edit operations were supplied", "name": name }));
    }

    warnings.extend(vet_sources(&mut report));
    report.name = name.clone();
    merge_provenance(&mut report, ctx);
    report.normalize();
    let html = render::render(&report);
    let dir = ctx.cfg.viz_dir();
    assert_inside(&dir, &html_path)?;
    std::fs::write(&html_path, &html)?;
    if ctx.cfg.viz.keep_model {
        std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    }
    if !ctx.session.created_reports.contains(&name) {
        ctx.session.created_reports.push(name.clone());
    }
    let m = report.metrics();
    Ok(json!({
        "ok": true,
        "name": name,
        "html_path": html_path.display().to_string(),
        "changes": changes,
        "stats": { "sources": m.sources, "items": m.items, "events": m.events, "relations": m.relations },
        "warnings": warnings,
    }))
}

/// What a finished report looks like after `finalize`, so the terminal can print
/// a one-line summary ("12 条主张 · 19 个来源 · 8 条已核实").
#[derive(Debug, Clone, Default)]
pub struct FinalizedReport {
    pub name: String,
    pub items: usize,
    pub sources: usize,
    pub verified: usize,
    pub flagged: usize,
    pub avg_reliability: f64,
}

/// Snapshot of everything the process knows about its own run, for the report's
/// audit section (engines, fetched URLs, blocked requests, tool log).
fn provenance_snapshot(ctx: &Ctx) -> Provenance {
    Provenance {
        generator: format!("VeriScout (ves) {}", crate::config::VERSION),
        tool_calls: ctx.session.tool_calls,
        agent_steps: 0,
        elapsed_ms: 0,
        engines: ctx.session.engines_used.iter().cloned().collect(),
        fetched_urls: ctx.session.fetched_urls.clone(),
        blocked: ctx.session.blocked_requests.clone(),
        blacklist_rules: ctx.cfg.search.blacklist.blocked.len(),
        steps: ctx.session.call_log.clone(),
    }
}

/// Refresh a report's provenance from the live session, but never *lose* evidence:
/// when a saved model is re-rendered in a fresh session (no calls yet), the stored
/// audit trail is kept instead of being overwritten with emptiness.
fn merge_provenance(report: &mut Report, ctx: &Ctx) {
    let fresh = provenance_snapshot(ctx);
    let old = std::mem::take(&mut report.provenance);
    report.provenance = fresh;
    if report.provenance.steps.is_empty() {
        report.provenance.steps = old.steps;
    }
    if report.provenance.tool_calls < old.tool_calls {
        report.provenance.tool_calls = old.tool_calls;
    }
    if report.provenance.agent_steps == 0 {
        report.provenance.agent_steps = old.agent_steps;
    }
    if report.provenance.elapsed_ms == 0 {
        report.provenance.elapsed_ms = old.elapsed_ms;
    }
    if report.provenance.engines.is_empty() {
        report.provenance.engines = old.engines;
    }
    if report.provenance.fetched_urls.is_empty() {
        report.provenance.fetched_urls = old.fetched_urls;
    }
    if report.provenance.blocked.is_empty() {
        report.provenance.blocked = old.blocked;
    }
}

/// Close the loop on a finished run: the model's closing answer and the complete
/// tool log are written into every report the run produced, then the HTML is
/// re-rendered. This is why the terminal can stay quiet — the detail lives here.
pub fn finalize(ctx: &mut Ctx, names: &[String], answer: &str, steps: u32, elapsed_ms: u128) -> Vec<FinalizedReport> {
    let mut out = Vec::new();
    let dir = ctx.cfg.viz_dir();
    for name in names {
        let (html_path, json_path) = paths(ctx, name);
        if !json_path.exists() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&json_path) else { continue };
        let Ok(mut report) = serde_json::from_str::<Report>(&raw) else { continue };

        // The closing answer is this run's 原话: it always becomes the report's
        // conclusion — even when the model left a placeholder ("（见最终回答）") or wrote
        // an inline version while creating the report. A substantial inline conclusion
        // is not discarded, it moves to the notes.
        let previous = report.conclusion.trim().to_string();
        let closing = answer.trim();
        if !closing.is_empty() {
            if !previous.is_empty() && previous != closing && previous.chars().count() >= 120 {
                report.notes.push(Note {
                    at: util::iso8601_now(),
                    text: format!("报告内结论（撰写版；§1「原话」已改用本次运行的收尾答复）：{previous}"),
                });
            }
            report.conclusion = closing.to_string();
        }
        merge_provenance(&mut report, ctx);
        report.provenance.agent_steps = steps;
        report.provenance.elapsed_ms = elapsed_ms as u64;
        report.normalize();

        let html = render::render(&report);
        if assert_inside(&dir, &html_path).is_err() || std::fs::write(&html_path, &html).is_err() {
            continue;
        }
        if ctx.cfg.viz.keep_model {
            if let Ok(model) = serde_json::to_string_pretty(&report) {
                let _ = std::fs::write(&json_path, model);
            }
        }

        let m = report.metrics();
        out.push(FinalizedReport {
            name: name.clone(),
            items: m.items,
            sources: m.sources,
            verified: report.items.iter().filter(|i| i.status == "verified" || i.status == "supported").count(),
            flagged: report.items.iter().filter(|i| i.status == "disputed" || i.status == "refuted").count(),
            avg_reliability: m.avg_reliability,
        });
    }
    out
}

fn merge_source(target: &mut Source, patch: &Source) {
    if !patch.title.is_empty() {
        target.title = patch.title.clone();
    }
    if !patch.url.is_empty() {
        target.url = patch.url.clone();
    }
    if !patch.kind.is_empty() {
        target.kind = patch.kind.clone();
    }
    if patch.reliability > 0 {
        target.reliability = patch.reliability;
    }
    if !patch.published.is_empty() {
        target.published = patch.published.clone();
    }
    if !patch.quote.is_empty() {
        target.quote = patch.quote.clone();
    }
    if !patch.engine.is_empty() {
        target.engine = patch.engine.clone();
    }
}

fn merge_item(target: &mut Item, patch: &Item) {
    if !patch.claim.is_empty() {
        target.claim = patch.claim.clone();
    }
    if !patch.detail.is_empty() {
        target.detail = patch.detail.clone();
    }
    if !patch.status.is_empty() {
        target.status = patch.status.clone();
    }
    if patch.confidence > 0 {
        target.confidence = patch.confidence;
    }
    if !patch.source_ids.is_empty() {
        target.source_ids = patch.source_ids.clone();
    }
    if !patch.tags.is_empty() {
        target.tags = patch.tags.clone();
    }
    if !patch.date.is_empty() {
        target.date = patch.date.clone();
    }
}

pub fn delete(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = Args::new(args);
    let raw = a.req_str("name")?;
    let dir = ctx.cfg.viz_dir();
    std::fs::create_dir_all(&dir).ok();

    if raw.trim() == "*" {
        if !a.bool("confirm_all", false) {
            return Err(anyhow!("deleting every report requires confirm_all=true"));
        }
        let mut removed = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("html") | Some("json") => {
                    assert_inside(&dir, &path)?;
                    std::fs::remove_file(&path)?;
                    removed.push(path.file_name().unwrap_or_default().to_string_lossy().to_string());
                }
                _ => {}
            }
        }
        ctx.session.created_reports.clear();
        return Ok(json!({ "ok": true, "deleted": removed, "count": removed.len() }));
    }

    let name = safe_name(&raw)?;
    let (html_path, json_path) = paths(ctx, &name);
    assert_inside(&dir, &html_path)?;
    let mut removed = Vec::new();
    for path in [html_path, json_path] {
        if path.exists() {
            std::fs::remove_file(&path)?;
            removed.push(path.file_name().unwrap_or_default().to_string_lossy().to_string());
        }
    }
    if removed.is_empty() {
        let existing = list_reports(ctx);
        return Err(anyhow!(
            "no report named '{name}'. Existing reports: {}",
            if existing.is_empty() { "none".to_string() } else { existing.join(", ") }
        ));
    }
    ctx.session.created_reports.retain(|n| n != &name);
    let count = removed.len();
    Ok(json!({ "ok": true, "deleted": removed, "count": count, "name": name }))
}

/// Reports currently on disk (injected into the system prompt so the model knows what exists).
pub fn list_reports(ctx: &Ctx) -> Vec<String> {
    let dir = ctx.cfg.viz_dir();
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("html") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn ctx_in(dir: &Path) -> Ctx {
        let mut cfg = Config::default();
        cfg.viz.dir = dir.display().to_string();
        Ctx::new(cfg)
    }

    #[test]
    fn name_slugs_are_path_safe() {
        assert!(safe_name("../../etc/passwd").is_ok());
        let n = safe_name("../../etc/passwd").unwrap();
        assert!(!n.contains('/') && !n.contains(".."));
        assert_eq!(safe_name("My Report!").unwrap(), "my-report");
    }

    #[test]
    fn create_then_edit_then_delete_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("ves-viz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);

        let created = create(
            &mut c,
            &json!({
                "name": "demo",
                "title": "Demo report",
                "query": "is this real?",
                "summary": "Summary text",
                "sources": [{ "id": "s1", "title": "Reuters", "url": "https://www.reuters.com/a", "kind": "secondary" }],
                "items": [{ "claim": "Something happened", "status": "verified", "source_ids": ["s1"] }],
                "events": [{ "id": "e1", "label": "Event A" }, { "id": "e2", "label": "Event B" }],
                "relations": [{ "from": "e1", "to": "e2", "kind": "causes" }]
            }),
        )
        .unwrap();
        assert_eq!(created["ok"], json!(true));
        let html_path = PathBuf::from(created["html_path"].as_str().unwrap());
        assert!(html_path.exists());
        assert!(std::fs::read_to_string(&html_path).unwrap().contains("Demo report"));

        let edited = edit(
            &mut c,
            &json!({
                "name": "demo",
                "set_title": "Demo report v2",
                "add_sources": [{ "id": "s2", "title": "NASA", "url": "https://www.nasa.gov/x", "kind": "official" }],
                "add_items": [{ "claim": "Second claim", "source_ids": ["s2"] }],
                "add_notes": ["a caveat"]
            }),
        )
        .unwrap();
        assert_eq!(edited["stats"]["sources"], json!(2));
        assert!(std::fs::read_to_string(&html_path).unwrap().contains("Demo report v2"));

        let deleted = delete(&mut c, &json!({ "name": "demo" })).unwrap();
        assert_eq!(deleted["count"], json!(2));
        assert!(!html_path.exists());

        let err = delete(&mut c, &json!({ "name": "demo" })).unwrap_err().to_string();
        assert!(err.contains("no report named"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn messy_model_payload_never_loses_claims() {
        let tmp = std::env::temp_dir().join(format!("ves-viz5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        // exactly the kind of payload a model produces: numbers as strings, a lone
        // string where a list is expected, claims keyed by object instead of array
        let out = create(
            &mut c,
            &json!({
                "name": "messy",
                "title": "Messy payload",
                "sources": [
                    { "id": "s1", "title": "Reuters", "url": "https://www.reuters.com/a", "reliability": "85%" }
                ],
                "items": {
                    "first": { "claim": "Claim one", "status": "verified", "confidence": "90", "source_ids": "s1" },
                    "second": { "claim": "Claim two", "status": "supported", "source_ids": ["s1"], "tags": "tag1" },
                    "broken": { "detail": "no claim field at all", "confidence": "not a number" }
                },
                "relations": [{ "from": "e1", "to": "e2", "kind": "causes", "strength": "4" }]
            }),
        )
        .unwrap();
        assert_eq!(out["stats"]["items"], json!(2), "both real claims must survive: {out}");
        assert_eq!(out["stats"]["sources"], json!(1));
        let model: Report = serde_json::from_str(&std::fs::read_to_string(out["model_path"].as_str().unwrap()).unwrap()).unwrap();
        let one = model.items.iter().find(|i| i.claim == "Claim one").expect("claim one kept");
        assert_eq!(one.confidence, 90, "numeric string must coerce");
        assert_eq!(one.source_ids, vec!["s1".to_string()], "lone string must coerce to list");
        let two = model.items.iter().find(|i| i.claim == "Claim two").expect("claim two kept");
        assert_eq!(two.tags, vec!["tag1".to_string()]);
        let warnings = out["warnings"].as_array().unwrap();
        assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("empty claim")), "{warnings:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn malformed_elements_are_reported_not_dropped_silently() {
        let tmp = std::env::temp_dir().join(format!("ves-viz6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        let out = create(
            &mut c,
            &json!({
                "name": "broken",
                "title": "Broken entry",
                "items": [ { "claim": "fine" }, { "confidence": 10 } ]
            }),
        )
        .unwrap();
        assert_eq!(out["stats"]["items"], json!(1));
        let warnings = out["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|w| w.as_str().unwrap().contains("items[1] was skipped")),
            "the dropped element must be reported with its index: {warnings:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finalize_writes_the_answer_and_the_tool_log_into_the_report() {
        let tmp = std::env::temp_dir().join(format!("ves-viz-final-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        c.session.tool_calls = 1;
        c.session.engines_used.insert("bing".into());
        c.session.call_log.push(CallStep {
            n: 1,
            tool: "web_search".into(),
            summary: "q=\"rust\" scope=web".into(),
            ok: true,
            ms: 120,
            bytes: 2048,
            note: String::new(),
        });
        let out = create(
            &mut c,
            &json!({
                "name": "final",
                "title": "Final",
                "items": [{ "claim": "A claim", "source_ids": ["1"] }],
                "sources": [{ "url": "https://www.reuters.com/x" }]
            }),
        )
        .unwrap();
        let name = out["name"].as_str().unwrap().to_string();

        let done = finalize(&mut c, &[name.clone()], "核心结论：该主张已被独立来源证实。", 3, 4321);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].items, 1);
        assert_eq!(done[0].sources, 1);
        assert_eq!(done[0].verified, 1);

        let model: Report = serde_json::from_str(&std::fs::read_to_string(paths(&c, &name).1).unwrap()).unwrap();
        assert!(model.conclusion.contains("核心结论"), "the closing answer must be stored");
        assert_eq!(model.provenance.agent_steps, 3);
        assert_eq!(model.provenance.elapsed_ms, 4321);
        assert_eq!(model.provenance.steps.len(), 1);
        assert_eq!(model.provenance.engines, vec!["bing".to_string()]);

        let html = std::fs::read_to_string(paths(&c, &name).0).unwrap();
        assert!(html.contains("核心结论"), "the final answer must be rendered into the HTML");
        assert!(html.contains("web_search"), "the tool log must be rendered into the HTML");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finalize_replaces_a_placeholder_conclusion_with_the_real_answer() {
        // A real run wrote "（见最终回答）" into the report; the injected answer was
        // skipped because the field was not empty, so the reader got the placeholder
        // instead of the model's own words. The closing answer must always win.
        let tmp = std::env::temp_dir().join(format!("ves-viz-placeholder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        let out = create(
            &mut c,
            &json!({
                "name": "placeholder",
                "title": "Placeholder",
                "conclusion": "（见最终回答）",
                "items": [{ "claim": "一条主张" }]
            }),
        )
        .unwrap();
        let name = out["name"].as_str().unwrap().to_string();
        finalize(&mut c, &[name.clone()], "完整答复：这次一共核实了 3 条说法。", 2, 100);
        let model: Report = serde_json::from_str(&std::fs::read_to_string(paths(&c, &name).1).unwrap()).unwrap();
        assert_eq!(model.conclusion, "完整答复：这次一共核实了 3 条说法。");
        assert!(!model.conclusion.contains("见最终回答"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finalize_keeps_a_long_inline_conclusion_as_a_note() {
        let tmp = std::env::temp_dir().join(format!("ves-viz-inline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let long = "模型在报告里自己写的长结论，讲清了背景、证据与不确定性。".repeat(8);
        let mut c = ctx_in(&tmp);
        let out = create(
            &mut c,
            &json!({
                "name": "inline",
                "title": "Inline",
                "conclusion": long,
                "items": [{ "claim": "一条主张" }]
            }),
        )
        .unwrap();
        let name = out["name"].as_str().unwrap().to_string();
        finalize(&mut c, &[name.clone()], "收尾答复：结论一句话。", 1, 10);
        let model: Report = serde_json::from_str(&std::fs::read_to_string(paths(&c, &name).1).unwrap()).unwrap();
        assert_eq!(model.conclusion, "收尾答复：结论一句话。");
        assert!(
            model.notes.iter().any(|n| n.text.contains("报告内结论")),
            "the inline version must be preserved as a note, got {:?}",
            model.notes
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rerendering_in_a_fresh_session_keeps_the_audit_trail() {
        let tmp = std::env::temp_dir().join(format!("ves-viz-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut first = ctx_in(&tmp);
        first.session.tool_calls = 5;
        first.session.call_log.push(CallStep {
            n: 5,
            tool: "curl_tool".into(),
            summary: "https://example.com/a".into(),
            ok: true,
            ms: 300,
            bytes: 900,
            note: String::new(),
        });
        create(&mut first, &json!({ "name": "keep", "title": "Keep" })).unwrap();

        // A brand-new session (e.g. replaying a saved model) must not wipe evidence.
        let mut second = ctx_in(&tmp);
        edit(
            &mut second,
            &json!({ "name": "keep", "set_summary": "updated", "set_conclusion": "收尾结论：已验证。" }),
        )
        .unwrap();
        let model: Report = serde_json::from_str(&std::fs::read_to_string(paths(&second, "keep").1).unwrap()).unwrap();
        assert_eq!(model.summary, "updated");
        assert_eq!(model.conclusion, "收尾结论：已验证。", "edit_html can set the conclusion directly");
        assert_eq!(model.provenance.steps.len(), 1, "tool log must survive a re-render");
        assert_eq!(model.provenance.tool_calls, 5, "call count must survive a re-render");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_non_http_sources() {
        let tmp = std::env::temp_dir().join(format!("ves-viz2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        let out = create(
            &mut c,
            &json!({
                "name": "x",
                "title": "X",
                "sources": [{ "url": "javascript:alert(1)" }, { "url": "https://ok.example/a" }]
            }),
        )
        .unwrap();
        assert_eq!(out["stats"]["sources"], json!(1));
        assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("non-http")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_never_clobbers() {
        let tmp = std::env::temp_dir().join(format!("ves-viz3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        let a = create(&mut c, &json!({ "name": "dup", "title": "A" })).unwrap();
        let b = create(&mut c, &json!({ "name": "dup", "title": "B" })).unwrap();
        assert_eq!(a["name"], json!("dup"));
        assert_eq!(b["name"], json!("dup-2"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn edit_requires_existing_model() {
        let tmp = std::env::temp_dir().join(format!("ves-viz4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mut c = ctx_in(&tmp);
        assert!(edit(&mut c, &json!({ "name": "ghost", "set_title": "x" })).is_err());
    }
}
