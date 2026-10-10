//! Rendering: turn a `Report` into one self-contained, offline HTML file.
//!
//! The file has no external dependencies (no CDN, no fonts, no network at all): the
//! correlation graph is a small hand-written force layout, so a report opened from
//! disk years later still renders exactly the same.

use crate::html::model::*;
use crate::util;
use serde_json::{json, Value};

const TEMPLATE: &str = include_str!("../../assets/template/report.html");

/// Baked in from Cargo.toml (`repository`) so the report links back to the project.
/// Empty value -> the template hides the GitHub button on its own.
const REPO: &str = env!("CARGO_PKG_REPOSITORY");

/// The project logo, inlined into the report as a favicon (offline: no CDN, no sidecar file).
const LOGO: &str = include_str!("../../assets/logo.svg");

/// Single-pass placeholder substitution: inserted values are never re-scanned,
/// so a claim containing literal `{{TITLE}}` cannot inject into the template.
fn fill(template: &str, pairs: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 8192);
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let key = &after[..end];
                match pairs.iter().find(|(k, _)| *k == key) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push_str("{{");
                        out.push_str(key);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn render(report: &Report) -> String {
    let m = report.metrics();
    let counts = report.status_counts();
    let verified: usize = counts
        .iter()
        .filter(|(status, _)| *status == "verified" || *status == "supported")
        .map(|(_, n)| *n)
        .sum();
    let pairs: Vec<(&str, String)> = vec![
        ("TITLE", esc(&report.title)),
        ("QUERY", esc(if report.query.is_empty() { &report.title } else { &report.query })),
        ("REPORT_NAME", esc(&report.name)),
        ("VERSION", crate::config::VERSION.to_string()),
        ("REPO_URL", esc(REPO)),
        ("REPO_HIDDEN", if REPO.trim().is_empty() { " hidden".into() } else { String::new() }),
        ("FAVICON", favicon_link()),
        ("THEME", esc(if report.theme == "light" { "light" } else { "dark" })),
        ("GENERATED", esc(&generated_label(report))),
        ("HEAD_STATS", head_stats_html(report, &m)),
        ("STATUS_CHIPS", status_chips_html(&counts, m.items)),
        ("LEDE", lede_html(&m, &counts)),
        ("SUMMARY", summary_html(report)),
        ("CONCLUSION", conclusion_html(report)),
        ("CLAIMS", claims_html(report)),
        ("TIMELINE", timeline_html(report)),
        ("SOURCES", sources_html(report)),
        ("AUDIT", audit_html(report, &m)),
        ("GRAPH_STATE", graph_state_html(report)),
        ("GRAPH_DATA", json_for_script(&graph(report))),
        ("KIND_COLORS", json_for_script(&kind_colors())),
        ("CLAIM_COUNT", m.items.to_string()),
        ("VERIFIED_COUNT", verified.to_string()),
        ("ITEM_COUNT", m.items.to_string()),
        ("EVENT_COUNT", m.events.to_string()),
        ("SOURCE_COUNT", m.sources.to_string()),
        ("RELATION_COUNT", m.relations.to_string()),
        ("TOOL_CALL_COUNT", report.provenance.tool_calls.to_string()),
    ];
    fill(TEMPLATE, &pairs)
}

fn esc(s: &str) -> String {
    util::html_escape(s)
}

/// `<link rel="icon">` built from the project logo, percent-encoded, so even the
/// tab icon is part of the single self-contained HTML file.
fn favicon_link() -> String {
    let mut enc = String::with_capacity(LOGO.len() * 2);
    for ch in LOGO.chars() {
        match ch {
            '%' => enc.push_str("%25"),
            '#' => enc.push_str("%23"),
            '"' => enc.push_str("%22"),
            '\'' => enc.push_str("%27"),
            '<' => enc.push_str("%3C"),
            '>' => enc.push_str("%3E"),
            '&' => enc.push_str("%26"),
            ' ' | '\n' | '\t' => enc.push_str("%20"),
            '\r' => {}
            c if (c as u32) < 0x80 => enc.push(c),
            c => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    enc.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    format!("<link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml,{enc}\">")
}

/// JSON that is safe to embed inside a `<script>` block.
fn json_for_script(v: &Value) -> String {
    let raw = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
    raw.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Local wall-clock time of the last render, derived from the stored UTC stamp.
fn generated_label(r: &Report) -> String {
    let stamp = if r.updated_at.is_empty() { &r.created_at } else { &r.updated_at };
    match util::parse_flexible_date(stamp) {
        Some(epoch) => util::iso8601_local(epoch),
        None if !stamp.trim().is_empty() => stamp.clone(),
        None => util::iso8601_now_local(),
    }
}

/// The numbers a reviewer checks first, as big tiles: a report should be skimmable
/// before it is read — the figures carry the page (deck.gallery report examples).
fn head_stats_html(r: &Report, m: &Metrics) -> String {
    let strong = r.items.iter().filter(|i| i.status == "verified" || i.status == "supported").count();
    let mut tiles: Vec<(String, &'static str)> = vec![
        (m.items.to_string(), "条说法"),
        (m.sources.to_string(), "个来源"),
    ];
    if m.items > 0 {
        tiles.push((strong.to_string(), "条有来源支持"));
    }
    if m.avg_reliability > 0.0 {
        tiles.push((format!("{:.0}", m.avg_reliability), "来源平均可信度"));
    }
    if !r.provenance.engines.is_empty() {
        tiles.push((r.provenance.engines.len().to_string(), "个搜索引擎"));
    }
    let body: Vec<String> = tiles
        .iter()
        .map(|(v, label)| format!("<div class=\"stat\"><b>{}</b><span>{}</span></div>", esc(v), esc(label)))
        .collect();
    format!("<div class=\"stat-row\">{}</div>", body.join(""))
}

/// One chip per review state — same order in every report, and every chip is a
/// filter button. The total is the only non-interactive chip.
fn status_chips_html(counts: &[(&'static str, usize)], total: usize) -> String {
    let mut out = format!("<span class=\"chip is-total\"><b>{total}</b> 条说法</span>");
    for (status, n) in counts {
        out.push_str(&format!(
            "<button class=\"chip s-{status}\" type=\"button\" data-status=\"{status}\" title=\"只看{label}的主张\"><span class=\"dot\" aria-hidden=\"true\"></span>{label} <b>{n}</b></button>",
            status = status,
            label = status_text(status),
            n = n
        ));
    }
    out
}

/// The layer-cake line: one sentence that says what the numbers add up to, so a
/// reviewer can decide whether to dig in before scrolling any further.
fn lede_html(m: &Metrics, counts: &[(&'static str, usize)]) -> String {
    let get = |k: &str| counts.iter().find(|(s, _)| *s == k).map(|(_, n)| *n).unwrap_or(0);
    if m.items == 0 {
        return esc("这次没有整理出可以逐条核对的结论；请以「资料来源」与「备注」为准。");
    }
    let strong = get("verified") + get("supported");
    let shaky = get("disputed") + get("refuted");
    let unverified = get("unverified");
    let mut s = format!("这份报告把问题拆成 {} 条说法逐条核对：{} 条有可靠来源支持", m.items, strong);
    if unverified > 0 {
        s.push_str(&format!("，{unverified} 条还没查实"));
    }
    if shaky > 0 {
        s.push_str(&format!("，{shaky} 条来源之间互相矛盾"));
    }
    s.push('。');
    if m.sources == 0 {
        s.push_str("这次没有登记到来源。");
    } else if m.primary_ratio > 0.0 {
        s.push_str(&format!(
            "共用到 {} 个来源，其中 {:.0}% 是一手、官方或学术材料，来源平均可信度 {:.0}/100。",
            m.sources,
            m.primary_ratio * 100.0,
            m.avg_reliability
        ));
    } else {
        s.push_str(&format!(
            "共用到 {} 个来源（暂无一手、官方或学术材料），来源平均可信度 {:.0}/100。",
            m.sources, m.avg_reliability
        ));
    }
    if m.dangling_refs > 0 {
        s.push_str(&format!("另有 {} 处引用找不到对应来源，已标出待复核。", m.dangling_refs));
    }
    esc(&s)
}

/// The model's own report summary (background and scope), when it wrote one.
fn summary_html(r: &Report) -> String {
    if r.summary.trim().is_empty() {
        return String::new();
    }
    format!(
        "<div class=\"box\"><div class=\"label\">模型写的摘要</div><div class=\"prose\">{}</div></div>",
        markdown_block(&r.summary)
    )
}

/// The closing answer of the run, verbatim — the model's 原话. The terminal does not
/// print it, so this is the only place to read what the model actually said.
fn conclusion_html(r: &Report) -> String {
    if r.conclusion.trim().is_empty() {
        return "<div class=\"box\"><div class=\"label\">原话 · 模型收尾答复</div><p class=\"hint\">这份报告里没有模型的原话（多半是用旧版本生成的）。重新运行一次就会自动补上；也可以在会话里让它用 edit_html 的 set_conclusion 手动补。</p></div>".to_string();
    }
    format!(
        "<div class=\"box\"><div class=\"label\">原话 · 模型收尾答复</div><div class=\"prose\">{}</div></div>",
        markdown_block(&r.conclusion)
    )
}

fn status_text(status: &str) -> &'static str {
    match status {
        "verified" => "已核实",
        "supported" => "有支持",
        "disputed" => "有矛盾",
        "refuted" => "被否认",
        _ => "未核实",
    }
}

fn human_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{:.0} 分 {:.0} 秒", (ms / 60_000) as f64, (ms % 60_000) as f64 / 1000.0)
    }
}

/// The audit section. This is where the terminal detail went: the whole tool
/// log, the URLs that were actually fetched, what the policy layer blocked, the
/// model's own caveats, and the numbers that make the run reproducible.
fn audit_html(r: &Report, m: &Metrics) -> String {
    let p = &r.provenance;
    let mut facts = String::new();
    let mut fact = |k: &str, v: String, s: String| {
        facts.push_str(&format!(
            "<div class=\"fact\"><dt>{}</dt><dd>{}<small>{}</small></dd></div>",
            esc(k),
            v,
            esc(&s)
        ));
    };
    fact(
        "工具调用次数",
        p.tool_calls.to_string(),
        format!("{} 步 · 用时 {}", p.agent_steps, if p.elapsed_ms == 0 { "—".to_string() } else { human_ms(p.elapsed_ms) }),
    );
    fact(
        "用过的搜索引擎",
        if p.engines.is_empty() { "—".to_string() } else { p.engines.len().to_string() },
        if p.engines.is_empty() { "未记录".to_string() } else { p.engines.join("、") },
    );
    fact(
        "打开过的网页",
        p.fetched_urls.len().to_string(),
        format!("另有 {} 次请求被安全策略拦下", p.blocked.len()),
    );
    fact("安全拦截规则", p.blacklist_rules.to_string(), "挡住已知的低质/有害域名".to_string());
    fact("来源平均可信度", format!("{:.0}", m.avg_reliability), "0–100 分，越高越可靠".to_string());
    fact("找不到来源的引用", m.dangling_refs.to_string(), "引用了不存在的来源编号".to_string());

    let mut out = format!("<dl class=\"facts\">{facts}</dl>");
    out.push_str(&tool_log_html(r));
    if !r.notes.is_empty() {
        let items: Vec<String> = r
            .notes
            .iter()
            .map(|n| {
                if n.at.trim().is_empty() {
                    format!("<li>{}</li>", markdown_inline(&n.text))
                } else {
                    format!("<li>{}<span class=\"tag\"> · {}</span></li>", markdown_inline(&n.text), esc(&n.at))
                }
            })
            .collect();
        out.push_str(&format!(
            "<details class=\"block\" open><summary>模型留下的备注（{} 条）</summary><div class=\"block-body\"><ul class=\"notes-list\">{}</ul></div></details>",
            r.notes.len(),
            items.join("")
        ));
    }
    if !p.fetched_urls.is_empty() {
        let items: Vec<String> = p.fetched_urls.iter().map(|u| format!("<li><code>{}</code></li>", esc(u))).collect();
        out.push_str(&format!(
            "<details class=\"block\"><summary>真正打开过的网址（{} 个）</summary><div class=\"block-body\"><ul class=\"plain\">{}</ul></div></details>",
            p.fetched_urls.len(),
            items.join("")
        ));
    }
    if !p.blocked.is_empty() {
        let items: Vec<String> = p.blocked.iter().map(|u| format!("<li><code>{}</code></li>", esc(u))).collect();
        out.push_str(&format!(
            "<details class=\"block\"><summary>被安全策略挡下的请求（{} 次）</summary><div class=\"block-body\"><ul class=\"plain warn\">{}</ul></div></details>",
            p.blocked.len(),
            items.join("")
        ));
    }
    out.push_str(&format!(
        "<div class=\"box\"><div class=\"label\">数据文件（可以继续编辑）</div><div class=\"prose\"><p>这份报告的原始数据存在同名的 <code>{}.json</code> 里；要继续补充内容，就在会话里让它用 <code>edit_html</code> 增删说法与来源。重新打开或重新渲染都不会覆盖上面的记录。</p></div></div>",
        esc(&r.name)
    ));
    out
}

/// The terminal transcript, verbatim: one row per tool call, in order.
fn tool_log_html(r: &Report) -> String {
    let p = &r.provenance;
    if p.steps.is_empty() {
        return "<details class=\"block\"><summary>每次工具调用的记录</summary><div class=\"block-body\"><p class=\"hint\">这次运行没有留下工具调用记录（报告可能是旧版本生成的）。</p></div></details>".to_string();
    }
    let rows: Vec<String> = p
        .steps
        .iter()
        .map(|s| {
            let ok = if s.ok {
                "<span class=\"badge-ok\">成功</span>".to_string()
            } else {
                "<span class=\"badge-bad\">失败</span>".to_string()
            };
            format!(
                "<tr><td class=\"num\">{n}</td><td><code>{tool}</code></td><td>{summary}</td><td>{ok}</td><td class=\"num\">{ms}</td><td class=\"num\">{bytes} B</td><td>{note}</td></tr>",
                n = s.n,
                tool = esc(&s.tool),
                summary = esc(&s.summary),
                ok = ok,
                ms = human_ms(s.ms),
                bytes = s.bytes,
                note = esc(&s.note),
            )
        })
        .collect();
    format!(
        "<details class=\"block\" open><summary>每次工具调用的记录（{} 次）</summary><div class=\"tbl-wrap flat\"><table><thead><tr><th class=\"num\">#</th><th>工具</th><th>参数</th><th>结果</th><th class=\"num\">用时</th><th class=\"num\">返回大小</th><th>备注</th></tr></thead><tbody>{}</tbody></table></div></details>",
        p.steps.len(),
        rows.join("")
    )
}

/// One line describing what the graph section actually contains.
fn graph_state_html(r: &Report) -> String {
    let nodes = if r.events.is_empty() { r.items.len() } else { r.events.len() };
    if nodes == 0 {
        return esc("暂无关系数据");
    }
    if r.relations.is_empty() {
        return esc(&format!("{nodes} 个节点 · 无显式关系（按共享来源连接）"));
    }
    esc(&format!("{nodes} 个节点 · {} 条关系", r.relations.len()))
}

// ---------------------------------------------------------------------------
// findings
// ---------------------------------------------------------------------------

/// Inline citation markers next to a claim: `[s1][s3]` linking to the table rows.
fn source_ref_links(r: &Report, ids: &[String]) -> String {
    if ids.is_empty() {
        return String::new();
    }
    let links: Vec<String> = ids
        .iter()
        .map(|sid| match r.sources.iter().find(|s| &s.id == sid) {
            Some(s) => format!(
                "<a href=\"#{id}\" title=\"{title}\">{id}</a>",
                id = esc(&s.id),
                title = esc(&s.title)
            ),
            None => format!("<span class=\"tag\">{}</span>", esc(sid)),
        })
        .collect();
    format!("<span class=\"refs\">{}</span>", links.join(""))
}

/// The claim ledger. One aligned row per claim — number, claim text, status,
/// confidence — with the evidence one disclosure away, never in the way.
fn claims_html(r: &Report) -> String {
    if r.items.is_empty() {
        return "<p class=\"hint\">没有记录任何主张。</p>".to_string();
    }
    let mut out = String::new();
    for (idx, item) in r.items.iter().enumerate() {
        let status = if STATUSES.contains(&item.status.as_str()) { item.status.as_str() } else { "unverified" };
        out.push_str(&format!(
            "<details class=\"claim s-{status}\" id=\"item-{id}\" data-status=\"{status}\">",
            status = status,
            id = esc(&item.id)
        ));
        out.push_str(&format!(
            "<summary><span class=\"claim-no\">{n}</span><span class=\"claim-text\">{claim}{refs}</span><span class=\"claim-status\"><span class=\"dot\" aria-hidden=\"true\"></span>{label}</span><span class=\"claim-conf\">{conf}%<span class=\"confbar\"><i style=\"width:{conf}%\"></i></span></span></summary>",
            n = idx + 1,
            claim = markdown_inline(&item.claim),
            refs = source_ref_links(r, &item.source_ids),
            label = status_text(status),
            conf = item.confidence,
        ));
        out.push_str("<div class=\"claim-body\">");
        if !item.detail.trim().is_empty() {
            out.push_str(&format!("<div class=\"detail prose\">{}</div>", markdown_block(&item.detail)));
        }
        out.push_str("<ul class=\"evidence\">");
        if item.source_ids.is_empty() {
            out.push_str("<li class=\"none\">没有挂来源：这条主张按未核实处理，不应直接引用。</li>");
        }
        for sid in &item.source_ids {
            match r.sources.iter().find(|s| &s.id == sid) {
                Some(s) => {
                    out.push_str("<li>");
                    out.push_str(&format!(
                        "<div class=\"src\"><a href=\"#{id}\"><b>[{id}] {title}</b></a><span class=\"kind\">{kind} · {domain}</span><a href=\"{url}\" target=\"_blank\" rel=\"noopener noreferrer\">原文 ↗</a></div>",
                        id = esc(&s.id),
                        title = markdown_inline(&s.title),
                        kind = esc(&s.kind),
                        domain = esc(&s.domain),
                        url = esc(&s.url),
                    ));
                    if !s.quote.trim().is_empty() {
                        out.push_str(&format!("<div class=\"quote\">{}</div>", esc(&s.quote)));
                    } else {
                        out.push_str("<div class=\"quote none\">该来源没有记录直接引文。</div>");
                    }
                    out.push_str("</li>");
                }
                None => out.push_str(&format!("<li class=\"none\">引用了未知来源编号 {}（需复核）</li>", esc(sid))),
            }
        }
        out.push_str("</ul>");
        let mut foot = String::new();
        if !item.date.is_empty() {
            foot.push_str(&format!("<span class=\"kind\">{}</span>", esc(&item.date)));
        }
        for tag in &item.tags {
            foot.push_str(&format!("<span class=\"tag\">#{}</span>", esc(tag)));
        }
        if !foot.is_empty() {
            out.push_str(&format!("<div class=\"claim-foot\">{foot}</div>"));
        }
        out.push_str("</div></details>");
    }
    out
}

// ---------------------------------------------------------------------------
// timeline
// ---------------------------------------------------------------------------

fn timeline_html(r: &Report) -> String {
    if r.events.is_empty() {
        return "<p class=\"hint\">未抽取到带时间的事件。</p>".to_string();
    }
    let mut dated: Vec<(&Event, i64)> = Vec::new();
    let mut undated: Vec<&Event> = Vec::new();
    for e in &r.events {
        match util::parse_flexible_date(&e.date) {
            Some(ts) if !e.date.trim().is_empty() => dated.push((e, ts)),
            _ => undated.push(e),
        }
    }
    dated.sort_by_key(|(_, ts)| *ts);
    let mut ordered: Vec<&Event> = dated.iter().map(|(e, _)| *e).collect();
    ordered.extend(undated);
    let mut out = String::new();
    for e in ordered {
        out.push_str("<div class=\"tl\">");
        if !e.date.is_empty() {
            out.push_str(&format!("<div class=\"tl-date\">{}</div>", esc(&e.date)));
        }
        out.push_str(&format!("<div class=\"tl-label\">{}</div>", markdown_inline(&e.label)));
        if !e.description.is_empty() {
            out.push_str(&format!("<div class=\"tl-desc\">{}</div>", markdown_inline(&e.description)));
        }
        if !e.source_ids.is_empty() {
            let links: Vec<String> = e
                .source_ids
                .iter()
                .filter_map(|sid| r.sources.iter().find(|s| &s.id == sid))
                .map(|s| format!("<a href=\"#{}\">[{}]</a>", esc(&s.id), esc(&s.id)))
                .collect();
            if !links.is_empty() {
                out.push_str(&format!("<div class=\"tl-refs\">{}</div>", links.join(" ")));
            }
        }
        out.push_str("</div>");
    }
    out
}

// ---------------------------------------------------------------------------
// sources
// ---------------------------------------------------------------------------

fn sources_html(r: &Report) -> String {
    if r.sources.is_empty() {
        return "<tr><td colspan=\"5\">没有收集到来源：这份报告是无来源草稿。</td></tr>".to_string();
    }
    let citations = r.citation_counts();
    let mut sorted: Vec<&Source> = r.sources.iter().collect();
    sorted.sort_by(|a, b| b.reliability.cmp(&a.reliability).then_with(|| a.domain.cmp(&b.domain)));
    let mut out = String::new();
    for s in sorted {
        out.push_str(&format!("<tr id=\"{}\">", esc(&s.id)));
        out.push_str(&format!("<td class=\"num\">{}</td>", esc(&s.id)));
        // source cell
        out.push_str("<td>");
        out.push_str(&format!(
            "<a class=\"src-title\" href=\"{url}\" target=\"_blank\" rel=\"noopener noreferrer\">{title}</a>",
            url = esc(&s.url),
            title = markdown_inline(&s.title)
        ));
        out.push_str(&format!("<div class=\"url\">{}</div>", esc(&s.url)));
        if !s.quote.trim().is_empty() {
            out.push_str(&format!(
                "<details><summary>证据引文</summary><blockquote>{}</blockquote></details>",
                esc(&s.quote)
            ));
        }
        out.push_str("</td>");
        // kind cell keeps the publication date with the type
        let meta = if s.published.trim().is_empty() {
            esc(&s.domain)
        } else {
            format!("{} · {}", esc(&s.domain), esc(&s.published))
        };
        out.push_str(&format!("<td><span class=\"kind\">{}</span><div class=\"url\">{}</div></td>", esc(&s.kind), meta));
        // reliability
        out.push_str(&format!(
            "<td><span class=\"reli\"><span class=\"bar\"><i style=\"width:{}%\"></i></span>{}</span></td>",
            s.reliability, s.reliability
        ));
        // cited by: the review loop in both directions
        let backlinks: Vec<String> = r
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.source_ids.iter().any(|id| id == &s.id))
            .map(|(idx, item)| format!("<a href=\"#item-{}\">#{}</a>", esc(&item.id), idx + 1))
            .collect();
        let cited = if backlinks.is_empty() {
            match citations.get(&s.id) {
                Some(_) => String::new(),
                None => "<span class=\"tag\">未被引用</span>".to_string(),
            }
        } else {
            backlinks.join(" ")
        };
        out.push_str(&format!("<td><span class=\"cited\">{cited}</span></td>"));
        out.push_str("</tr>");
    }
    out
}

// ---------------------------------------------------------------------------
// correlation graph
// ---------------------------------------------------------------------------

fn kind_colors() -> Value {
    json!({
        "event": "var(--accent)",
        "claim": "var(--supported)",
        "actor": "var(--unverified)",
        "cause": "var(--disputed)",
        "effect": "var(--verified)",
        "source": "var(--text-mute)"
    })
}

/// Build the graph. When the model supplied no events, derive a claim/source
/// co-occurrence graph so the visualization is never empty.
fn graph(r: &Report) -> Value {
    if !r.events.is_empty() {
        let nodes: Vec<Value> = r
            .events
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "label": e.label,
                    "kind": if kind_colors().get(&e.kind).is_some() { e.kind.clone() } else { "event".to_string() },
                    "date": e.date,
                    "description": util::truncate_chars(&e.description, 180).0,
                    "sources": e.source_ids,
                })
            })
            .collect();
        let resolve = |raw: &str| -> Option<String> {
            let key = raw.trim().to_ascii_lowercase();
            if key.is_empty() {
                return None;
            }
            r.events
                .iter()
                .find(|e| e.id.to_ascii_lowercase() == key || e.label.to_ascii_lowercase() == key)
                .map(|e| e.id.clone())
        };
        let links: Vec<Value> = r
            .relations
            .iter()
            .filter_map(|rel| {
                let s = resolve(&rel.from)?;
                let t = resolve(&rel.to)?;
                if s == t {
                    return None;
                }
                Some(json!({ "source": s, "target": t, "kind": rel.kind, "note": rel.note, "strength": rel.strength }))
            })
            .collect();
        return json!({ "nodes": nodes, "links": links });
    }

    // fallback: claims linked when they share a source
    let nodes: Vec<Value> = r
        .items
        .iter()
        .map(|i| {
            json!({
                "id": i.id,
                "label": util::truncate_chars(&i.claim, 60).0,
                "kind": "claim",
                "date": i.date,
                "description": util::truncate_chars(&i.detail, 180).0,
                "sources": i.source_ids,
            })
        })
        .collect();
    let mut links: Vec<Value> = Vec::new();
    for (idx, a) in r.items.iter().enumerate() {
        for b in r.items.iter().skip(idx + 1) {
            let shared = a.source_ids.iter().filter(|s| b.source_ids.contains(s)).count();
            if shared > 0 {
                links.push(json!({ "source": a.id, "target": b.id, "kind": "related", "note": format!("共享 {shared} 个来源"), "strength": shared.min(3) }));
            }
        }
    }
    json!({ "nodes": nodes, "links": links })
}

// ---------------------------------------------------------------------------
// tiny markdown subset
// ---------------------------------------------------------------------------

/// Inline formatting: **bold**, `code` - everything is escaped first.
pub fn markdown_inline(text: &str) -> String {
    let mut out = esc(text);
    out = replace_pairs(&out, "**", "<strong>", "</strong>");
    out = replace_code(&out);
    out.replace("  \n", "<br>")
}

fn replace_pairs(input: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut open_next = true;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos]);
        out.push_str(if open_next { open } else { close });
        open_next = !open_next;
        rest = &rest[pos + marker.len()..];
    }
    out.push_str(rest);
    out
}

fn replace_code(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut open = true;
    while let Some(pos) = rest.find('\u{60}') {
        out.push_str(&rest[..pos]);
        out.push_str(if open { "<code>" } else { "</code>" });
        open = !open;
        rest = &rest[pos + 1..];
    }
    out.push_str(rest);
    out
}

/// Block formatting: paragraphs, `- ` bullets, `1. ` numbering, `# `…`###### `
/// headings, `> ` quotes and `---` rules. The model writes markdown, so the renderer
/// has to understand the shapes it actually emits — a literal `#` left in the report
/// makes the whole thing look unfinished.
pub fn markdown_block(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    fn close_list(out: &mut String, list: &mut &str) {
        if !list.is_empty() {
            out.push_str("</");
            out.push_str(list);
            out.push('>');
            *list = "";
        }
    }
    let mut out = String::new();
    let mut list = ""; // "" | "ul" | "ol"
    let mut in_quote = false;
    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            close_list(&mut out, &mut list);
            if in_quote {
                out.push_str("</blockquote>");
                in_quote = false;
            }
            continue;
        }
        if trimmed.chars().count() >= 3 && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') {
            close_list(&mut out, &mut list);
            if in_quote {
                out.push_str("</blockquote>");
                in_quote = false;
            }
            out.push_str("<hr>");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ").or_else(|| trimmed.strip_prefix(">")) {
            close_list(&mut out, &mut list);
            if !in_quote {
                out.push_str("<blockquote>");
                in_quote = true;
            }
            out.push_str(&format!("<p>{}</p>", markdown_inline(rest.trim())));
            continue;
        }
        if in_quote {
            out.push_str("</blockquote>");
            in_quote = false;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
            close_list(&mut out, &mut list);
            let rest = trimmed[hashes + 1..].trim_start();
            let tag = if hashes <= 2 { "h3" } else { "h4" };
            out.push_str(&format!("<{tag}>{}</{tag}>", markdown_inline(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            if list != "ul" {
                close_list(&mut out, &mut list);
                out.push_str("<ul>");
                list = "ul";
            }
            out.push_str(&format!("<li>{}</li>", markdown_inline(rest)));
            continue;
        }
        if let Some(rest) = ordered_item(trimmed) {
            if list != "ol" {
                close_list(&mut out, &mut list);
                out.push_str("<ol>");
                list = "ol";
            }
            out.push_str(&format!("<li>{}</li>", markdown_inline(rest)));
            continue;
        }
        close_list(&mut out, &mut list);
        out.push_str(&format!("<p>{}</p>", markdown_inline(trimmed)));
    }
    close_list(&mut out, &mut list);
    if in_quote {
        out.push_str("</blockquote>");
    }
    out
}

/// `1. text` / `12) text` → `Some("text")`.
fn ordered_item(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some(rest.trim_start())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let mut r = Report {
            name: "demo".into(),
            title: "Demo <script>alert(1)</script>".into(),
            query: "q".into(),
            summary: "First paragraph.\n\n- bullet one\n- bullet two".into(),
            conclusion: "第一段结论。\n\n- 要点一：已验证\n- 要点二：仍存疑".into(),
            items: vec![Item {
                claim: "Claim with **bold** and <img src=x onerror=1>".into(),
                status: "verified".into(),
                source_ids: vec!["s1".into(), "sX".into()],
                ..Default::default()
            }],
            sources: vec![Source {
                url: "https://www.reuters.com/x?a=1&b=2".into(),
                title: "Reuters piece".into(),
                quote: "quoted \"text\"".into(),
                ..Default::default()
            }],
            events: vec![
                Event { label: "A".into(), date: "2024-01-01".into(), ..Default::default() },
                Event { label: "B".into(), date: "2025-05-05".into(), ..Default::default() },
            ],
            relations: vec![Relation { from: "A".into(), to: "B".into(), kind: "causes".into(), ..Default::default() }],
            provenance: Provenance {
                tool_calls: 2,
                agent_steps: 3,
                elapsed_ms: 12_400,
                engines: vec!["bing".into(), "wikipedia".into()],
                fetched_urls: vec!["https://www.reuters.com/x?a=1&b=2".into()],
                blocked: vec!["https://spam.example/ad".into()],
                steps: vec![
                    CallStep { n: 1, tool: "web_search".into(), summary: "q=\"demo\" scope=web".into(), ok: true, ms: 320, bytes: 4096, note: String::new() },
                    CallStep { n: 2, tool: "curl_tool".into(), summary: "https://www.reuters.com/x".into(), ok: false, ms: 900, bytes: 120, note: "HTTP 403".into() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        r.normalize();
        r
    }

    #[test]
    fn renders_offline_single_file() {
        let html = render(&report());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        assert!(!html.contains("http://cdn"), "must not reference a CDN");
        assert!(!html.contains("<script src="), "no external scripts allowed");
        assert!(!html.contains("href=\"https://cdn"));
        assert!(html.contains("reuters.com"));
        assert!(html.contains("https://www.reuters.com/x?a=1&amp;b=2"));
    }

    #[test]
    fn escapes_hostile_content() {
        let html = render(&report());
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<img src=x onerror=1>"));
    }

    #[test]
    fn graph_json_is_embedded_and_script_safe() {
        let html = render(&report());
        assert!(html.contains("GRAPH = {"));
        assert!(html.contains("\"nodes\""));
        assert!(html.contains("\"links\""));
        assert!(!html.contains("</script><script>"));
    }

    #[test]
    fn placeholders_are_all_replaced() {
        let html = render(&report());
        for leftover in [
            "{{TITLE}}",
            "{{QUERY}}",
            "{{REPORT_NAME}}",
            "{{GENERATED}}",
            "{{HEAD_STATS}}",
            "{{STATUS_CHIPS}}",
            "{{LEDE}}",
            "{{SUMMARY}}",
            "{{CONCLUSION}}",
            "{{CLAIMS}}",
            "{{TIMELINE}}",
            "{{SOURCES}}",
            "{{AUDIT}}",
            "{{GRAPH_STATE}}",
            "{{GRAPH_DATA}}",
            "{{KIND_COLORS}}",
            "{{CLAIM_COUNT}}",
            "{{VERIFIED_COUNT}}",
            "{{EVENT_COUNT}}",
            "{{SOURCE_COUNT}}",
            "{{RELATION_COUNT}}",
            "{{TOOL_CALL_COUNT}}",
            "{{VERSION}}",
            "{{REPO_URL}}",
            "{{REPO_HIDDEN}}",
            "{{FAVICON}}",
            "{{THEME}}",
        ] {
            assert!(!html.contains(leftover), "unreplaced placeholder {leftover}");
        }
    }

    #[test]
    fn version_repo_and_logo_are_wired_in() {
        let html = render(&report());
        assert!(html.contains(crate::config::VERSION), "version must be stamped into the page");
        assert!(html.contains("data:image/svg+xml,"), "logo favicon must be inlined, not linked");
        assert!(html.contains("id=\"repo-link\""), "repo button must exist in the markup");
        assert!(html.contains("rel=\"noopener noreferrer\""));
        if REPO.trim().is_empty() {
            assert!(html.contains("class=\"btn gh hidden\""), "empty repository must hide the button");
        } else {
            assert!(html.contains(&format!("href=\"{REPO}\"")), "repository url must be wired in");
            assert!(!html.contains("class=\"btn gh hidden\""), "configured repository must show the button");
        }
    }

    #[test]
    fn markdown_handles_the_shapes_models_actually_write() {
        let html = markdown_block(
            "# 大标题\n\n正文一句。\n\n- 要点一\n- 要点二\n\n1. 第一步\n2. 第二步\n\n> 引文一句\n\n---\n\n## 小标题",
        );
        assert!(html.contains("<h3>大标题</h3>"), "{html}");
        assert!(html.contains("<ul><li>要点一</li><li>要点二</li></ul>"), "{html}");
        assert!(html.contains("<ol><li>第一步</li><li>第二步</li></ol>"), "{html}");
        assert!(html.contains("<blockquote><p>引文一句</p></blockquote>"), "{html}");
        assert!(html.contains("<hr>"), "{html}");
        assert!(html.contains("<h3>小标题</h3>"), "{html}");
        assert!(!html.contains("# 大标题"), "raw markdown must not survive into the page");
    }

    #[test]
    fn claims_are_a_review_ledger() {
        let html = render(&report());
        assert!(html.contains("<details class=\"claim s-verified\" id=\"item-i1\" data-status=\"verified\">"), "{html}");
        assert!(html.contains("class=\"claim-status\""), "status must carry a text label, not only colour");
        assert!(html.contains("已核实"));
        assert!(html.contains("class=\"confbar\""), "confidence needs a visible bar");
        assert!(html.contains("href=\"#s1\""), "claim must link to its source row");
        assert!(html.contains("<tr id=\"s1\">"), "source rows must be anchor targets");
        assert!(html.contains("class=\"evidence\""), "evidence must be one disclosure away");
        assert!(html.contains("引用了未知来源编号 sX"), "dangling source refs must be flagged for review");
    }

    #[test]
    fn conclusion_and_audit_trail_are_rendered() {
        let html = render(&report());
        assert!(html.contains("原话 · 模型收尾答复"), "the closing answer needs its own block");
        assert!(html.contains("第一段结论"), "the injected answer must appear verbatim");
        assert!(html.contains("每次工具调用的记录"), "the tool transcript must be in the report");
        assert!(html.contains("web_search"), "tool names must be in the transcript");
        assert!(html.contains("HTTP 403"), "errors must be preserved in the transcript");
        assert!(html.contains("被安全策略挡下的请求"), "blocked requests need their own block");
        assert!(html.contains("真正打开过的网址"));
        assert!(html.contains("12.4 s"), "elapsed time must be human readable");
    }

    #[test]
    fn review_affordances_are_wired() {
        let html = render(&report());
        for needle in [
            "id=\"status-chips\"",
            "data-status=\"verified\"",
            "id=\"claim-search\"",
            "id=\"claim-status\"",
            "id=\"toggle-evidence\"",
            "id=\"source-search\"",
            "id=\"progress-bar\"",
            "id=\"print-report\"",
            "id=\"theme-toggle\"",
            "id=\"density-toggle\"",
            "data-nav=\"claims\"",
        ] {
            assert!(html.contains(needle), "missing review affordance: {needle}");
        }
    }

    #[test]
    fn typography_follows_the_reading_rules() {
        let html = render(&report());
        assert!(html.contains("--fs:16px"), "body text must be 16px");
        assert!(html.contains("--lh:1.75"), "line height must stay generous");
        assert!(html.contains("--measure:42em"), "prose must be capped at ≈40 CJK glyphs per line");
        assert!(!html.contains("text-align:justify"), "text must never be justified (WCAG 1.4.8)");
        assert!(html.contains("@media print"), "the audit artefact must print cleanly");
        assert!(html.contains("prefers-reduced-motion"), "motion must respect user settings");
    }

    /// Dev helper: write a filled-in report to /tmp so the layout can be looked at
    /// in a real browser (and screenshotted):
    ///   cargo test -- --ignored write_layout_preview
    #[test]
    #[ignore]
    fn write_layout_preview() {
        let mut r = Report {
            name: "layout-preview".into(),
            title: "Rust 数据竞争与编译期检查：一次溯源核查".into(),
            query: "Rust 的所有权模型能否消除数据竞争？有哪些权威来源支持这一点？".into(),
            summary: "本报告核查 Rust 所有权/借用检查在编译期消除数据竞争这一主张，覆盖官方文档、学术论文与社区实践三类来源。".into(),
            conclusion: "**核心结论（置信度 88%）**：Rust 的类型系统在安全子集内可以静态消除数据竞争；这一点有官方文档与多篇同行评审论文支持。\n\n- 要点一：所有权 + 借用检查在编译期阻止别名可变引用，因此数据竞争无法通过编译（来源 [s1]、[s2]）。\n- 要点二：`unsafe` 代码与 FFI 边界不受此保证覆盖，需要额外审查（来源 [s3]）。\n- 要点三：跨线程共享仍可能产生死锁等逻辑问题，类型系统不检查这类错误（来源 [s4]）。\n\n- 不确定性：部分社区基准测量的对象与论文不同，不能直接比较。\n- 报告文件：layout-preview.html".into(),
            items: (1..=8)
                .map(|i| {
                    let (claim, status, conf, srcs, detail) = match i {
                        1 => ("Rust 在编译期拒绝存在数据竞争的安全代码", "verified", 92, vec!["s1", "s2"], "官方文档明确说明借用检查器在编译期阻止别名可变引用；形式化论文给出证明。"),
                        2 => ("`unsafe` 代码与 FFI 不在类型系统保证范围内", "verified", 90, vec!["s3"], "官方文档的安全边界说明要求开发者自行维护不变式。"),
                        3 => ("所有权模型无法防止死锁这类逻辑错误", "supported", 74, vec!["s4"], "教材与社区文档均指出类型系统不检查锁顺序。"),
                        4 => ("Rust 程序因此不再需要任何运行时数据竞争检查", "disputed", 45, vec!["s5", "s6"], "争议集中在跨进程共享内存与 unsafe 边界：运行时检查仍可能必要。"),
                        5 => ("所有 Rust 项目都应强制启用 unsafe 代码审计", "refuted", 12, vec!["s6"], "惯例建议按 unsafe 面积决定审计强度，而非一概而论。"),
                        _ => ("待核查子命题：内存模型与原子序号的默认语义", "unverified", 40, vec![], "尚未找到直接引用该细节的权威来源，按未核实处理。"),
                    };
                    Item {
                        id: format!("i{i}"),
                        claim: claim.into(),
                        detail: detail.into(),
                        status: status.into(),
                        confidence: conf,
                        source_ids: srcs.iter().map(|s| s.to_string()).collect(),
                        tags: if i == 1 { vec!["memory-safety".into(), "compiler".into()] } else { vec![] },
                        date: if i % 2 == 0 { "2026-03-11".into() } else { String::new() },
                    }
                })
                .collect(),
            sources: vec![
                ("s1", "The Rust Reference · Aliasing and borrow checking", "https://doc.rust-lang.org/reference/", "official", 95, "References and borrows are checked so aliasing with mutation is prevented at compile time."),
                ("s2", "RustBelt: Securing the Foundations of the Rust Programming Language", "https://plv.mpi-sws.org/rustbelt/", "academic", 94, "The type system of Rust guarantees memory safety of safe code."),
                ("s3", "The Rustonomicon · Unsafe code guidelines", "https://doc.rust-lang.org/nomicon/", "official", 92, "Unsafe code must maintain the invariants the compiler relies on."),
                ("s4", "Why Rust's type system does not prevent deadlocks", "https://blog.example.org/rust-deadlock/", "secondary", 61, "Lock ordering is a runtime property, not a type-level one."),
                ("s5", "Cross-process shared memory in Rust", "https://example.com/cross-proc", "secondary", 58, "Shared memory outside the process boundary is not protected by borrow checking."),
                ("s6", "Community discussion: when to audit unsafe code", "https://forum.example.net/t/unsafe-audit", "social", 35, "Audit effort should follow the amount of unsafe surface."),
            ]
            .into_iter()
            .map(|(id, title, url, kind, reliability, quote)| Source {
                id: id.into(),
                title: title.into(),
                url: url.into(),
                kind: kind.into(),
                reliability,
                quote: quote.into(),
                published: "2026-01-20".into(),
                ..Default::default()
            })
            .collect(),
            events: vec![
                Event { id: "e1".into(), label: "Rust 1.0 发布，所有权模型进入主线".into(), date: "2015-05-15".into(), description: "借用检查器成为默认编译期检查。".into(), ..Default::default() },
                Event { id: "e2".into(), label: "RustBelt 论文发表，形式化验证安全子集".into(), date: "2018-07-01".into(), description: "证明 safe Rust 的内存安全与线程安全。".into(), source_ids: vec!["s2".into()], ..Default::default() },
                Event { id: "e3".into(), label: "unsafe 指南与审计实践成型".into(), date: "2021-11-03".into(), description: "社区形成按面积审计 unsafe 的惯例。".into(), source_ids: vec!["s3".into(), "s6".into()], ..Default::default() },
            ],
            relations: vec![
                Relation { from: "e1".into(), to: "e2".into(), kind: "follows".into(), strength: 3, ..Default::default() },
                Relation { from: "e2".into(), to: "e3".into(), kind: "causes".into(), strength: 2, ..Default::default() },
            ],
            notes: vec![
                Note { at: "2026-03-11".into(), text: "两条子命题缺少权威来源，已按未核实标记。".into() },
                Note { at: String::new(), text: "社区基准与论文口径不同，未做直接比较。".into() },
            ],
            provenance: Provenance {
                tool_calls: 14,
                agent_steps: 9,
                elapsed_ms: 96_400,
                engines: vec!["bing".into(), "wikipedia".into(), "arxiv".into(), "hackernews".into()],
                fetched_urls: vec!["https://doc.rust-lang.org/reference/".into(), "https://plv.mpi-sws.org/rustbelt/".into()],
                blocked: vec!["https://spam.example/ad".into()],
                steps: (1..=6u64)
                    .map(|n| CallStep {
                        n: n as u32,
                        tool: if n % 3 == 0 { "curl_tool" } else { "web_search" }.into(),
                        summary: if n % 3 == 0 { "https://doc.rust-lang.org/nomicon/".into() } else { "q=\"rust data race\" scope=academic".into() },
                        ok: n != 5,
                        ms: 320 * n,
                        bytes: 4096 * n,
                        note: if n == 5 { "HTTP 403: blocked by origin".into() } else { String::new() },
                    })
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        };
        r.normalize();
        std::fs::write("/tmp/ves-layout-preview.html", render(&r)).unwrap();
    }

    #[test]
    fn hostile_text_cannot_inject_placeholders() {
        let mut r = report();
        r.title = "{{SUMMARY}}".into();
        r.summary = "safe".into();
        let html = render(&r);
        assert!(html.contains("{{SUMMARY}}"), "literal placeholder text must survive untouched");
        assert!(html.contains("<p>safe</p>"));
    }

    #[test]
    fn markdown_subset() {
        assert_eq!(markdown_inline("a **b** c"), "a <strong>b</strong> c");
        assert_eq!(markdown_inline("x `y` z"), "x <code>y</code> z");
        let block = markdown_block("## H\ntext\n- a\n- b");
        assert!(block.contains("<h3>H</h3>"));
        assert!(block.contains("<ul><li>a</li><li>b</li></ul>"));
    }
}
