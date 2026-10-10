//! Multi-engine search aggregation: fan out, blacklist-filter, dedupe, rank.

pub mod providers;

use crate::blacklist::{trust_prior, Matcher};
use crate::config::Config;
use crate::http::Http;
use crate::util;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// General web + encyclopedic sources.
    Web,
    /// Recent news and press coverage.
    News,
    /// Peer-reviewed / preprint literature.
    Academic,
    /// Code, repositories and developer discussion.
    Code,
    /// Q&A and community discussion.
    Qa,
    /// Everything that is enabled.
    All,
}

impl Scope {
    pub fn parse(s: &str) -> Scope {
        match s.to_ascii_lowercase().as_str() {
            "news" => Scope::News,
            "academic" | "paper" | "papers" | "scholar" => Scope::Academic,
            "code" | "repo" | "dev" => Scope::Code,
            "qa" | "forum" | "community" => Scope::Qa,
            "all" => Scope::All,
            _ => Scope::Web,
        }
    }

    fn allows(self, engine: &str) -> bool {
        let general = matches!(
            engine,
            "anysearch" | "bing" | "duckduckgo" | "wikipedia" | "searx" | "mojeek"
        );
        let vertical = matches!(
            engine,
            "googlenews" | "crossref" | "openalex" | "arxiv" | "hackernews" | "github" | "stackexchange"
        );
        let keyed = matches!(engine, "tavily" | "serper" | "brave" | "custom");
        match self {
            Scope::Web => general || keyed,
            Scope::News => engine == "googlenews" || engine == "bing" || keyed,
            Scope::Academic => matches!(engine, "crossref" | "openalex" | "arxiv" | "wikipedia") || keyed,
            Scope::Code => matches!(engine, "github" | "hackernews" | "stackexchange") || keyed,
            Scope::Qa => matches!(engine, "stackexchange" | "hackernews" | "searx") || keyed,
            Scope::All => general || vertical || keyed,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::Web => "web",
            Scope::News => "news",
            Scope::Academic => "academic",
            Scope::Code => "code",
            Scope::Qa => "qa",
            Scope::All => "all",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
    pub domain: String,
    /// 0-100 source-trust prior (institutional press / academic / gov / edu score higher).
    pub trust: i32,
    /// Final ranking score after cross-engine agreement is taken into account.
    pub score: f64,
    /// How many independent engines returned this URL.
    pub corroboration: usize,
    pub published: Option<String>,
    /// Extra provenance: venue, authors, stars, points, ...
    pub extra: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    pub engine: String,
    pub ok: bool,
    pub results: usize,
    pub ms: u128,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReport {
    pub query: String,
    pub scope: String,
    pub results: Vec<SearchResult>,
    pub engines: Vec<EngineStatus>,
    pub filtered: Vec<String>,
    pub took_ms: u128,
    pub notes: Vec<String>,
}

pub struct Aggregator<'a> {
    pub cfg: &'a Config,
    pub http: &'a Http,
}

impl<'a> Aggregator<'a> {
    pub fn new(cfg: &'a Config, http: &'a Http) -> Self {
        Self { cfg, http }
    }

    /// Which engines are usable right now (key-based ones need a credential).
    pub fn active_engines(&self, scope: Scope, requested: Option<&[String]>) -> Vec<String> {
        let mut out = Vec::new();
        for name in self.cfg.all_engines() {
            let name = name.trim().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            if let Some(req) = requested {
                if !req.iter().any(|r| r.eq_ignore_ascii_case(&name)) {
                    continue;
                }
            } else if !scope.allows(&name) {
                continue;
            }
            if !self.engine_available(&name) {
                continue;
            }
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    pub fn engine_available(&self, name: &str) -> bool {
        match name {
            // available only when the local CLI is actually present
            "anysearch" => providers::anysearch::resolve(self.cfg).is_some(),
            "tavily" => self.cfg.search_api_key(&self.cfg.search.tavily_api_key_env).is_some(),
            "serper" => self.cfg.search_api_key(&self.cfg.search.serper_api_key_env).is_some(),
            "brave" => self.cfg.search_api_key(&self.cfg.search.brave_api_key_env).is_some(),
            "custom" => !self.cfg.search.custom_endpoint.trim().is_empty(),
            _ => true,
        }
    }

    /// Fan out to every active engine in parallel, then filter / dedupe / rank.
    pub fn search(&self, query: &str, scope: Scope, limit: usize, requested: Option<&[String]>) -> SearchReport {
        let started = Instant::now();
        let engines = self.active_engines(scope, requested);
        let per_engine = self.cfg.search.results_per_engine.max(1);
        let mut collected: Vec<(String, Result<Vec<SearchResult>, String>, u128)> = Vec::new();

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for engine in &engines {
                let engine = engine.clone();
                handles.push(s.spawn(move || {
                    let t0 = Instant::now();
                    let out = providers::run(&engine, self.http, self.cfg, query, per_engine);
                    (engine, out.map_err(|e| e.to_string()), t0.elapsed().as_millis())
                }));
            }
            for h in handles {
                match h.join() {
                    Ok(v) => collected.push(v),
                    Err(_) => collected.push(("unknown".into(), Err("engine thread panicked".into()), 0)),
                }
            }
        });

        let mut statuses = Vec::new();
        let mut pool: Vec<SearchResult> = Vec::new();
        let mut filtered: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for (engine, outcome, ms) in collected {
            match outcome {
                Ok(results) => {
                    statuses.push(EngineStatus {
                        engine: engine.clone(),
                        ok: true,
                        results: results.len(),
                        ms,
                        error: None,
                    });
                    if results.is_empty() {
                        notes.push(format!("{engine}: reachable but returned no parseable results"));
                    }
                    pool.extend(results);
                }
                Err(e) => {
                    statuses.push(EngineStatus {
                        engine: engine.clone(),
                        ok: false,
                        results: 0,
                        ms,
                        error: Some(e.clone()),
                    });
                    notes.push(format!("{engine}: {e}"));
                }
            }
        }

        // ---- blacklist + shape filtering -------------------------------------
        let matcher = Matcher::new(&self.cfg.search.blacklist);
        let mut kept = Vec::new();
        for mut r in pool {
            if r.url.trim().is_empty() || r.title.trim().is_empty() {
                continue;
            }
            let url = util::normalize_url(&r.url);
            let host = match util::host_of(&url) {
                Some(h) => h,
                None => continue,
            };
            if let Some(reason) = matcher.check_host(&host) {
                filtered.push(format!("{} ({})", host, reason));
                continue;
            }
            r.url = url;
            r.domain = host;
            r.trust = trust_prior(&r.domain);
            kept.push(r);
        }

        // ---- dedupe, merge corroboration -------------------------------------
        let mut merged: Vec<SearchResult> = Vec::new();
        for r in kept {
            let key = dedupe_key(&r.url);
            if let Some(existing) = merged.iter_mut().find(|m| dedupe_key(&m.url) == key || similar_title(&m.title, &r.title)) {
                existing.corroboration += 1;
                if existing.snippet.chars().count() < r.snippet.chars().count() {
                    existing.snippet = r.snippet.clone();
                }
                if existing.published.is_none() {
                    existing.published = r.published.clone();
                }
                if !existing.engine.split(',').any(|e| e == r.engine) {
                    existing.engine = format!("{},{}", existing.engine, r.engine);
                }
                continue;
            }
            merged.push(r);
        }

        // ---- score ------------------------------------------------------------
        for r in merged.iter_mut() {
            let mut score = r.trust as f64 * 0.55;
            score += match r.engine.split(',').next().unwrap_or("") {
                "anysearch" => 14.0,
                "wikipedia" => 16.0,
                "bing" => 10.0,
                "openalex" | "crossref" | "arxiv" => 12.0,
                "googlenews" => 8.0,
                "searx" => 6.0,
                "tavily" | "serper" | "brave" => 6.0,
                _ => 4.0,
            };
            score += (r.corroboration.saturating_sub(1) as f64) * 9.0;
            score += term_overlap(query, &r.title) * 26.0;
            score += term_overlap(query, &r.snippet) * 10.0;
            if let Some(p) = &r.published {
                if let Some(ts) = util::parse_flexible_date(p) {
                    let age_days = (util::now_epoch() - ts) as f64 / 86_400.0;
                    if age_days >= 0.0 && age_days < 90.0 {
                        score += (90.0 - age_days) / 9.0;
                    }
                }
            }
            if r.url.starts_with("https://") {
                score += 2.0;
            }
            if r.snippet.trim().is_empty() {
                score -= 6.0;
            }
            r.score = (score * 100.0).round() / 100.0;
        }

        merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let cap = if limit == 0 { self.cfg.search.max_results } else { limit };
        merged.truncate(cap);
        filtered.sort();
        filtered.dedup();
        statuses.sort_by(|a, b| a.engine.cmp(&b.engine));

        SearchReport {
            query: query.to_string(),
            scope: scope.label().to_string(),
            results: merged,
            engines: statuses,
            filtered,
            took_ms: started.elapsed().as_millis(),
            notes,
        }
    }
}

pub fn dedupe_key(url: &str) -> String {
    let u = util::normalize_url(url);
    let no_scheme = u.split("://").nth(1).unwrap_or(&u).to_string();
    let trimmed = no_scheme.trim_end_matches('/').to_string();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains(".wikipedia.org/wiki/") {
        // treat language variants and tracking-parameter variants of one article as a single source
        let article = lower.split("/wiki/").nth(1).unwrap_or("");
        let article = article.split(['?', '#']).next().unwrap_or(article);
        return format!("wikipedia:{}", article.trim_end_matches('/'));
    }
    lower
}

fn similar_title(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    };
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b && a.chars().count() > 12
}

fn term_overlap(query: &str, text: &str) -> f64 {
    let q: Vec<String> = tokenize(query);
    if q.is_empty() {
        return 0.0;
    }
    let t = tokenize(text);
    let hits = q.iter().filter(|w| t.iter().any(|x| x == *w)).count();
    hits as f64 / q.len() as f64
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lower = s.to_lowercase();
    // CJK: use character bigrams so Chinese queries still score
    let cjk: Vec<char> = lower.chars().filter(|c| util::is_cjk(*c)).collect();
    for w in cjk.windows(2) {
        let g: String = w.iter().collect();
        if !out.contains(&g) {
            out.push(g);
        }
    }
    for w in lower.split(|c: char| !c.is_alphanumeric()) {
        if w.chars().count() > 1 && !out.iter().any(|x| x == w) {
            out.push(w.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_gating() {
        assert!(Scope::Web.allows("bing"));
        assert!(!Scope::Web.allows("crossref"));
        assert!(Scope::Academic.allows("crossref"));
        assert!(!Scope::News.allows("github"));
        assert!(Scope::All.allows("arxiv"));
        assert_eq!(Scope::parse("papers"), Scope::Academic);
    }

    #[test]
    fn wikipedia_variants_share_a_key() {
        assert_eq!(
            dedupe_key("https://en.wikipedia.org/wiki/Rust_(programming_language)?x=1"),
            dedupe_key("http://en.wikipedia.org/wiki/Rust_(programming_language)")
        );
    }

    #[test]
    fn title_similarity_ignores_punctuation() {
        assert!(similar_title("Rust (programming language)", "Rust programming language!"));
        assert!(!similar_title("Rust", "Python"));
    }

    #[test]
    fn cjk_queries_tokenize_into_bigrams() {
        let t = tokenize("验证信息来源");
        assert!(t.contains(&"验证".to_string()));
        assert!(t.len() >= 3);
    }
}
