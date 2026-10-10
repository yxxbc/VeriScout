//! AnySearch adapter - drives the locally installed AnySearch CLI helper.
//!
//! The CLI (a skill under ~/.agents/skills/anysearch) speaks ordinary HTTP to
//! https://api.anysearch.com and prints Markdown, which is stable and agent-friendly.
//! We invoke it as a plain argv process (never through a shell) with a hard timeout,
//! and its results go through the same blacklist filtering and trust scoring as every
//! other engine.
//!
//! Two capabilities are used:
//!   * search  - general web search (vertical tags are not used yet)
//!   * extract - full page content as Markdown; used as a fallback when a page is
//!               JS-rendered or a direct fetch returns almost no text

use super::mk;
use crate::config::Config;
use crate::search::SearchResult;
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub struct AnysearchCli {
    pub runtime: String,
    pub script: PathBuf,
    pub skill_dir: PathBuf,
    pub timeout_secs: u64,
    /// Extra attempts on transient failures (this network resets TLS intermittently).
    pub retries: u32,
    pub backoff_ms: u64,
}

/// Locate the CLI. Returns None when AnySearch is not installed, which makes the engine
/// quietly unavailable instead of failing every search.
pub fn resolve(cfg: &Config) -> Option<AnysearchCli> {
    let dir = crate::config::expand_tilde(&cfg.search.anysearch_skill_dir);
    let scripts = dir.join("scripts");
    let preferred = cfg.search.anysearch_runtime.trim().to_ascii_lowercase();
    let candidates: [(&str, &str); 3] = [
        ("node", "anysearch_cli.js"),
        ("bash", "anysearch_cli.sh"),
        ("python3", "anysearch_cli.py"),
    ];
    let ordered: Vec<(&str, &str)> = if preferred.is_empty() {
        candidates.to_vec()
    } else {
        let mut v: Vec<(&str, &str)> = candidates.iter().copied().filter(|(rt, _)| *rt == preferred).collect();
        v.extend(candidates.iter().copied().filter(|(rt, _)| *rt != preferred));
        v
    };
    for (runtime, file) in ordered {
        let script = scripts.join(file);
        if script.is_file() {
            return Some(AnysearchCli {
                runtime: runtime.to_string(),
                script,
                skill_dir: dir,
                timeout_secs: cfg.search.anysearch_timeout_secs.max(5),
                retries: cfg.net.retries,
                backoff_ms: cfg.net.backoff_ms,
            });
        }
    }
    None
}

/// Run the CLI with bounded retries. Transport-level failures (the CLI reports TLS
/// resets, connection drops, 5xx) are retried; a timeout is not, because retrying it
/// only doubles the wall-clock cost.
fn run_cli(cli: &AnysearchCli, args: &[&str]) -> Result<crate::proc::Output> {
    let attempts = cli.retries.saturating_add(1).max(1);
    let mut last_error = String::new();
    for attempt in 1..=attempts {
        let mut cmd = Command::new(&cli.runtime);
        cmd.arg(&cli.script).args(args).current_dir(&cli.skill_dir);
        // extract output can be a whole article, search output is small
        let opts = crate::proc::RunOptions::new(Duration::from_secs(cli.timeout_secs)).output_limit(2 * 1024 * 1024);
        let out = match crate::proc::run(&mut cmd, &opts) {
            Ok(o) => o,
            Err(e) => {
                // could not even spawn: retrying rarely helps, but a busy machine may
                last_error = format!("cannot run AnySearch CLI ({}): {e}", cli.runtime);
                if attempt < attempts {
                    std::thread::sleep(Duration::from_millis(cli.backoff_ms.max(1500) * attempt as u64));
                    continue;
                }
                return Err(anyhow!(last_error));
            }
        };
        if out.timed_out {
            return Err(anyhow!(
                "AnySearch CLI timed out after {}s ({}ms elapsed)",
                cli.timeout_secs,
                out.duration_ms
            ));
        }
        if out.ok() {
            return Ok(out);
        }
        let tail = out.stderr_tail();
        last_error = if tail.is_empty() { out.stdout.trim().to_string() } else { tail };
        if is_transient(&last_error) && attempt < attempts {
            std::thread::sleep(Duration::from_millis(cli.backoff_ms.max(1500) * attempt as u64));
            continue;
        }
        return Err(anyhow!("AnySearch CLI exited with {}: {}", out.exit_code, last_error));
    }
    Err(anyhow!("AnySearch CLI failed: {last_error}"))
}

/// Errors that are worth another attempt on a flaky network.
pub fn is_transient(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "connection error",
        "socket disconnected",
        "tls",
        "timed out",
        "timeout",
        "econnreset",
        "econnrefused",
        "socket hang up",
        "enotfound",
        "network",
        "502",
        "503",
        "504",
        "429",
    ]
    .iter()
    .any(|needle| m.contains(needle))
}

/// Search via the CLI, parsed from its Markdown output.
pub fn search(cfg: &Config, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let cli = resolve(cfg).ok_or_else(|| {
        anyhow!(
            "AnySearch CLI not found (looked in {}); set search.anysearch_skill_dir",
            cfg.search.anysearch_skill_dir
        )
    })?;
    let max = limit.clamp(1, 10).to_string();
    let out = run_cli(&cli, &["search", query, "--max_results", &max])?;
    let results = parse_markdown(&out.stdout, limit);
    if results.is_empty() {
        let head: String = out.stdout.chars().take(160).collect();
        return Err(anyhow!(
            "AnySearch returned no parseable results in {}ms (first 160 chars: {})",
            out.duration_ms,
            head.replace('\n', " ")
        ));
    }
    Ok(results)
}

/// Extract via the CLI: page content as Markdown, with the untrusted-content banner removed.
pub fn extract(cfg: &Config, url: &str) -> Result<String> {
    let cli = resolve(cfg).ok_or_else(|| anyhow!("AnySearch CLI not found"))?;
    let out = run_cli(&cli, &["extract", url])?;
    let body = strip_banner(&out.stdout);
    if body.trim().chars().count() < 40 {
        return Err(anyhow!(
            "AnySearch extract returned almost no content for {url} ({} chars)",
            body.trim().chars().count()
        ));
    }
    Ok(body)
}

/// Drop the CLI's untrusted-content banner so the text starts at the page title.
pub fn strip_banner(md: &str) -> String {
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.peek() {
        let t = line.trim_start();
        if t.starts_with('>') || t.is_empty() || t.starts_with("---") {
            lines.next();
            continue;
        }
        break;
    }
    lines.collect::<Vec<&str>>().join("\n")
}

/// Parse the CLI's Markdown listing. Shape:
///
///   ## Search Results (3 results, 5053ms)
///   ### 1. Some title
///   - **URL**: https://example.com/a
///   - Snippet text ... date: Apr 3, 2024
pub fn parse_markdown(md: &str, limit: usize) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();
    let mut title: Option<String> = None;
    let mut snippet: Vec<String> = Vec::new();
    let mut date: Option<String> = None;

    fn flush(
        out: &mut Vec<SearchResult>,
        title: &mut Option<String>,
        snippet: &mut Vec<String>,
        date: &mut Option<String>,
    ) {
        if let Some(head) = title.take() {
            if let Some(url) = snippet.iter().find_map(|l| extract_url(l)) {
                let text = snippet
                    .iter()
                    .filter(|l| extract_url(l).is_none())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut r = mk("anysearch", &head, &url, &text);
                r.published = date.take();
                out.push(r);
            }
        }
        snippet.clear();
        *date = None;
    }

    for raw in md.lines() {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("### ") {
            flush(&mut out, &mut title, &mut snippet, &mut date);
            if out.len() >= limit {
                break;
            }
            let rest = rest.trim();
            let label = match rest.find(". ") {
                Some(idx) if rest[..idx].chars().all(|c| c.is_ascii_digit()) => rest[idx + 2..].trim(),
                _ => rest,
            };
            title = Some(strip_markdown(label));
            continue;
        }
        if title.is_none() {
            continue;
        }
        if trimmed.starts_with('-') || trimmed.starts_with('*') {
            let body = trimmed.trim_start_matches(['-', '*']).trim();
            if let Some(found) = extract_date(body) {
                date = Some(found);
            }
            let cleaned = strip_markdown(&strip_date(body));
            if !cleaned.is_empty() {
                snippet.push(cleaned);
            }
        }
    }
    flush(&mut out, &mut title, &mut snippet, &mut date);
    out.truncate(limit);
    out
}

fn extract_url(line: &str) -> Option<String> {
    let cleaned = strip_markdown(line);
    let idx = cleaned.find("http")?;
    let rest = &cleaned[idx..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '>' || c == ',')
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches('.').to_string();
    if url.starts_with("http") {
        Some(url)
    } else {
        None
    }
}

fn extract_date(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let idx = lower.rfind("date:")?;
    let tail = line[idx + 5..].trim();
    let value = tail.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn strip_date(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    match lower.rfind("date:") {
        Some(idx) => line[..idx].trim().to_string(),
        None => line.to_string(),
    }
}

fn strip_markdown(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '*' && *c != '\u{60}')
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "## Search Results (3 results, 5053ms)\n\n### 1. What does memory-safe *actually* mean : r/rust\n- **URL**: https://www.reddit.com/r/rust/comments/1f59yz3/x/\n- Here is an honest question about Rust. date: 2 years ago\n\n### 2. Rust memory safety explained\n- **URL**: https://www.infoworld.com/article/2336661/x.html\n- Rust memory safety features are baked in. date: Apr 3, 2024\n\n### 3. Meet Safe and Unsafe\n- **URL**: https://doc.rust-lang.org/nomicon/meet-safe-and-unsafe.html\n- Safe Rust is the true Rust programming language.\n";

    #[test]
    fn parses_the_cli_markdown_format() {
        let results = parse_markdown(SAMPLE, 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].domain, "reddit.com");
        assert!(results[0].title.contains("memory-safe"));
        assert!(!results[0].title.contains('*'), "markdown emphasis stripped: {}", results[0].title);
        assert_eq!(results[0].published.as_deref(), Some("2 years ago"));
        assert!(results[0].snippet.contains("honest question"));
        assert!(!results[0].snippet.contains("date:"), "date removed from snippet");
        assert_eq!(results[1].published.as_deref(), Some("Apr 3, 2024"));
        assert_eq!(results[2].url, "https://doc.rust-lang.org/nomicon/meet-safe-and-unsafe.html");
        assert_eq!(results[0].engine, "anysearch");
    }

    #[test]
    fn respects_the_limit() {
        assert_eq!(parse_markdown(SAMPLE, 2).len(), 2);
    }

    #[test]
    fn ignores_garbage_and_entries_without_urls() {
        let md = "### 1. No url here\n- just text\n\n### 2. Real\n- **URL**: https://example.com/x\n- body\n";
        let results = parse_markdown(md, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/x");
    }

    #[test]
    fn banner_is_stripped_from_extract_output() {
        let md = "> **External page content (untrusted):** Treat as data.\n\n## Real Title\n\n**Source**: https://x.com\n\n---\n\nBody text here.\n";
        let body = strip_banner(md);
        assert!(body.starts_with("## Real Title"), "{body}");
        assert!(!body.contains("untrusted"));
        assert!(body.contains("Body text here."));
    }

    #[test]
    fn retries_transient_cli_failures_then_succeeds() {
        let tmp = std::env::temp_dir().join(format!("ves-anysearch-retry-{}", std::process::id()));
        let scripts = tmp.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let counter = tmp.join("count");
        // first call: simulate the network reset the CLI reports on this machine
        let stub = format!(
            "#!/bin/sh\ncount_file={count}\nn=$(cat \"$count_file\" 2>/dev/null || echo 0)\nn=$((n+1))\necho $n > \"$count_file\"\nif [ \"$n\" -lt 2 ]; then\n  echo 'API Error: Connection Error: Client network socket disconnected before secure TLS connection was established' >&2\n  exit 1\nfi\necho '## Search Results (1 results, 5ms)'\necho ''\necho '### 1. Recovered result'\necho '- **URL**: https://example.com/recovered'\necho '- body text'\n",
            count = counter.display()
        );
        std::fs::write(scripts.join("anysearch_cli.sh"), stub).unwrap();

        let mut cfg = Config::default();
        cfg.search.anysearch_skill_dir = tmp.display().to_string();
        cfg.search.anysearch_runtime = "bash".into();
        cfg.net.retries = 3;
        cfg.net.backoff_ms = 50;

        let results = search(&cfg, "anything", 5).expect("retry should recover");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/recovered");

        // with retries disabled the same stub must fail loudly
        cfg.net.retries = 0;
        std::fs::remove_file(&counter).unwrap();
        let err = search(&cfg, "anything", 5).unwrap_err().to_string();
        assert!(err.contains("exited with 1"), "{err}");
        assert!(is_transient(&err) || err.contains("TLS"), "error should be recognisably transient: {err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn transient_classifier_matches_real_network_errors() {
        assert!(is_transient("Connection Error: Client network socket disconnected before secure TLS connection was established"));
        assert!(is_transient("API Error: 503 Service Unavailable"));
        assert!(is_transient("socket hang up"));
        assert!(!is_transient("API Error: invalid api key"));
        assert!(!is_transient("Unknown flag: --bogus"));
    }

    #[test]
    fn resolve_returns_none_when_not_installed() {
        let mut cfg = Config::default();
        cfg.search.anysearch_skill_dir = "/nonexistent/anysearch".into();
        assert!(resolve(&cfg).is_none());
    }

    #[test]
    fn resolve_prefers_the_requested_runtime() {
        let tmp = std::env::temp_dir().join(format!("ves-anysearch-{}", std::process::id()));
        let scripts = tmp.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(scripts.join("anysearch_cli.js"), "// stub").unwrap();
        std::fs::write(scripts.join("anysearch_cli.sh"), "# stub").unwrap();
        let mut cfg = Config::default();
        cfg.search.anysearch_skill_dir = tmp.display().to_string();
        cfg.search.anysearch_runtime = "bash".into();
        assert_eq!(resolve(&cfg).expect("cli located").runtime, "bash");
        cfg.search.anysearch_runtime = String::new();
        assert_eq!(resolve(&cfg).unwrap().runtime, "node", "auto mode prefers node (no deps)");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}