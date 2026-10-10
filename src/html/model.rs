//! The report data model. The HTML file is only a projection of this structure, which is
//! why edit_html can update a report reliably instead of doing text surgery.

use crate::blacklist::trust_prior;
use crate::util;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Report {
    pub name: String,
    pub title: String,
    pub query: String,
    pub created_at: String,
    pub updated_at: String,
    pub theme: String,
    pub locale: String,
    pub summary: String,
    /// The model's closing answer, injected when the run ends: the terminal does not
    /// echo it, so the full reasoning and its caveats live here (report §1 结论).
    pub conclusion: String,
    pub items: Vec<Item>,
    pub sources: Vec<Source>,
    pub events: Vec<Event>,
    pub relations: Vec<Relation>,
    pub notes: Vec<Note>,
    pub provenance: Provenance,
}

/// Accept a number, a numeric string ("85%"), or a boolean-ish value as i32.
pub fn flex_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        Value::String(s) => s
            .trim()
            .trim_end_matches('%')
            .parse::<f64>()
            .map(|f| f.round() as i32)
            .unwrap_or(0),
        Value::Bool(b) => b as i32 * 100,
        _ => 0,
    })
}

/// Accept a string, a number, or an array of either as Vec<String>.
pub fn flex_vec_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::String(s) => {
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
        Value::Number(n) => vec![n.to_string()],
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) if !s.trim().is_empty() => Some(s),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Value::Object(map) => map
            .into_iter()
            .map(|(_, v)| match v {
                Value::String(s) => s,
                other => other.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// Accept anything textual-ish as String (null -> empty).
pub fn flex_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Item {
    pub id: String,
    pub claim: String,
    pub detail: String,
    pub status: String,
    #[serde(default, deserialize_with = "flex_i32")]
    pub confidence: i32,
    #[serde(default, deserialize_with = "flex_vec_string")]
    pub source_ids: Vec<String>,
    #[serde(default, deserialize_with = "flex_vec_string")]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "flex_string")]
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub url: String,
    pub domain: String,
    pub kind: String,
    #[serde(default, deserialize_with = "flex_i32")]
    pub reliability: i32,
    pub published: String,
    pub quote: String,
    pub engine: String,
    pub retrieved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Event {
    pub id: String,
    pub label: String,
    pub date: String,
    pub description: String,
    pub kind: String,
    #[serde(default, deserialize_with = "flex_vec_string")]
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Relation {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub note: String,
    #[serde(default, deserialize_with = "flex_i32")]
    pub strength: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Note {
    pub at: String,
    pub text: String,
}

/// One tool invocation, kept so the report carries the audit trail that used to
/// scroll past in the terminal: what was called, with which arguments, whether
/// it succeeded, how long it took and how big the answer was.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CallStep {
    pub n: u32,
    pub tool: String,
    pub summary: String,
    pub ok: bool,
    pub ms: u64,
    pub bytes: u64,
    /// Short error text (or a note such as "结果被截断") when there is one.
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Provenance {
    pub generator: String,
    pub tool_calls: u32,
    /// Agent loop steps (model turns), and how long the run took.
    pub agent_steps: u32,
    pub elapsed_ms: u64,
    pub engines: Vec<String>,
    pub fetched_urls: Vec<String>,
    pub blocked: Vec<String>,
    pub blacklist_rules: usize,
    /// Every tool call of the run, in order.
    pub steps: Vec<CallStep>,
}

#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub sources: usize,
    pub items: usize,
    pub events: usize,
    pub relations: usize,
    pub verified_ratio: f64,
    pub avg_reliability: f64,
    pub primary_ratio: f64,
    pub dangling_refs: usize,
}

pub const STATUSES: [&str; 5] = ["verified", "supported", "unverified", "disputed", "refuted"];
pub const SOURCE_KINDS: [&str; 6] = ["primary", "secondary", "academic", "official", "social", "unknown"];
pub const RELATION_KINDS: [&str; 6] = ["causes", "supports", "contradicts", "follows", "related", "mentions"];

pub fn normalize_status(raw: &str, has_sources: bool) -> String {
    let s = raw.trim().to_ascii_lowercase();
    let mapped = match s.as_str() {
        "verified" | "confirmed" | "true" => "verified",
        "supported" | "likely" | "mostly-true" => "supported",
        "disputed" | "contested" | "conflicting" => "disputed",
        "refuted" | "false" | "debunked" => "refuted",
        "unverified" | "unknown" | "unconfirmed" | "" => "unverified",
        _ => "unverified",
    };
    if mapped == "unverified" && has_sources && s.is_empty() {
        return "supported".into();
    }
    mapped.to_string()
}

pub fn default_confidence(status: &str) -> i32 {
    match status {
        "verified" => 90,
        "supported" => 72,
        "disputed" => 45,
        "refuted" => 12,
        _ => 40,
    }
}

pub fn normalize_kind(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    if SOURCE_KINDS.contains(&s.as_str()) {
        s
    } else {
        match s.as_str() {
            "journal" | "paper" | "preprint" => "academic".into(),
            "gov" | "government" | "institution" => "official".into(),
            "press" | "news" | "media" => "secondary".into(),
            "blog" | "forum" | "twitter" | "weibo" => "social".into(),
            "dataset" | "document" | "report" => "primary".into(),
            _ => "unknown".into(),
        }
    }
}

pub fn kind_reliability_bonus(kind: &str) -> i32 {
    match kind {
        "official" => 12,
        "academic" => 12,
        "primary" => 8,
        "secondary" => 0,
        "social" => -12,
        _ => 0,
    }
}

pub fn normalize_relation_kind(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    if RELATION_KINDS.contains(&s.as_str()) {
        return s;
    }
    match s.as_str() {
        "cause" | "caused" | "leads-to" | "result" => "causes".into(),
        "support" | "supports" | "confirms" => "supports".into(),
        "contradict" | "contradicts" | "conflicts" | "opposes" => "contradicts".into(),
        "follow" | "follows" | "after" | "before" => "follows".into(),
        "mention" | "mentions" | "references" => "mentions".into(),
        _ => "related".into(),
    }
}

impl Report {
    /// Fill in ids, timestamps, defaults and derived fields. Idempotent.
    pub fn normalize(&mut self) {
        if self.name.is_empty() {
            self.name = util::slugify(&self.title);
        }
        if self.created_at.is_empty() {
            self.created_at = util::iso8601_now();
        }
        self.updated_at = util::iso8601_now();
        if self.theme.is_empty() {
            self.theme = "dark".into();
        }
        if self.locale.is_empty() {
            self.locale = "zh".into();
        }
        if self.provenance.generator.is_empty() {
            self.provenance.generator = format!("VeriScout (ves) {}", crate::config::VERSION);
        }

        // sources: stable ids, domain, kind, reliability prior, dedupe by URL
        let mut seen: Vec<String> = Vec::new();
        let mut kept: Vec<Source> = Vec::new();
        for (i, s) in self.sources.iter().enumerate() {
            let mut s = s.clone();
            if s.id.trim().is_empty() {
                s.id = format!("s{}", i + 1);
            }
            s.url = s.url.trim().to_string();
            if s.domain.is_empty() {
                s.domain = util::host_of(&s.url).unwrap_or_default();
            }
            s.kind = normalize_kind(&s.kind);
            if s.reliability <= 0 {
                let base = trust_prior(&s.domain);
                s.reliability = (base + kind_reliability_bonus(&s.kind)).clamp(5, 98);
            } else {
                s.reliability = s.reliability.clamp(0, 100);
            }
            if s.title.trim().is_empty() {
                s.title = if s.domain.is_empty() { s.url.clone() } else { s.domain.clone() };
            }
            if s.retrieved_at.is_empty() {
                s.retrieved_at = self.created_at.clone();
            }
            let key = util::normalize_url(&s.url).to_ascii_lowercase();
            if !s.url.is_empty() && seen.contains(&key) {
                continue;
            }
            if !key.is_empty() {
                seen.push(key);
            }
            kept.push(s);
        }
        self.sources = kept;

        // items
        let known: Vec<String> = self.sources.iter().map(|s| s.id.clone()).collect();
        let mut dangling = 0usize;
        for (i, item) in self.items.iter_mut().enumerate() {
            if item.id.trim().is_empty() {
                item.id = format!("i{}", i + 1);
            }
            // accept source references given as ids, urls or 1-based indexes
            let mut resolved: Vec<String> = Vec::new();
            for raw in item.source_ids.clone() {
                let r = raw.trim().to_string();
                if known.contains(&r) {
                    resolved.push(r);
                    continue;
                }
                if let Some(found) = self
                    .sources
                    .iter()
                    .find(|s| util::normalize_url(&s.url) == util::normalize_url(&r) || s.url == r)
                {
                    resolved.push(found.id.clone());
                    continue;
                }
                if let Ok(idx) = r.parse::<usize>() {
                    if idx >= 1 && idx <= self.sources.len() {
                        resolved.push(self.sources[idx - 1].id.clone());
                        continue;
                    }
                }
                dangling += 1;
                resolved.push(r);
            }
            resolved.dedup();
            item.source_ids = resolved;
            item.status = normalize_status(&item.status, !item.source_ids.is_empty());
            if item.confidence <= 0 {
                item.confidence = default_confidence(&item.status);
            }
            item.confidence = item.confidence.clamp(0, 100);
        }

        // events
        for (i, e) in self.events.iter_mut().enumerate() {
            if e.id.trim().is_empty() {
                e.id = format!("e{}", i + 1);
            }
            if e.kind.trim().is_empty() {
                e.kind = "event".into();
            }
            e.kind = e.kind.trim().to_ascii_lowercase();
            e.source_ids = e
                .source_ids
                .iter()
                .map(|r| {
                    let r = r.trim();
                    if known.contains(&r.to_string()) {
                        r.to_string()
                    } else if let Ok(idx) = r.parse::<usize>() {
                        if idx >= 1 && idx <= self.sources.len() {
                            return self.sources[idx - 1].id.clone();
                        }
                        r.to_string()
                    } else {
                        r.to_string()
                    }
                })
                .collect();
        }

        // relations: keep only those that resolve to a node
        let node_keys: Vec<String> = self
            .events
            .iter()
            .flat_map(|e| vec![e.id.to_ascii_lowercase(), e.label.to_ascii_lowercase()])
            .collect();
        let previous = std::mem::take(&mut self.relations);
        self.relations = previous
            .into_iter()
            .filter(|r| {
                let f = r.from.trim().to_ascii_lowercase();
                let t = r.to.trim().to_ascii_lowercase();
                !f.is_empty() && !t.is_empty() && f != t && node_keys.contains(&f) && node_keys.contains(&t)
            })
            .map(|mut r| {
                r.kind = normalize_relation_kind(&r.kind);
                r.strength = if r.strength <= 0 { 2 } else { r.strength.clamp(1, 5) };
                r
            })
            .collect();

        // notes
        if self.notes.is_empty() {
            self.notes.push(Note {
                at: self.created_at.clone(),
                text: "本报告由自动检索与交叉验证生成；可靠性评分为来源先验启发式分值，不构成事实判断。".into(),
            });
        }
        for n in self.notes.iter_mut() {
            if n.at.is_empty() {
                n.at = self.created_at.clone();
            }
        }
        let _ = dangling;
    }

    pub fn metrics(&self) -> Metrics {
        let verified = self
            .items
            .iter()
            .filter(|i| i.status == "verified" || i.status == "supported")
            .count();
        let avg_rel = if self.sources.is_empty() {
            0.0
        } else {
            self.sources.iter().map(|s| s.reliability as f64).sum::<f64>() / self.sources.len() as f64
        };
        let primary = self
            .sources
            .iter()
            .filter(|s| matches!(s.kind.as_str(), "primary" | "official" | "academic"))
            .count();
        let dangling = self
            .items
            .iter()
            .flat_map(|i| i.source_ids.iter())
            .filter(|id| !self.sources.iter().any(|s| &s.id == *id))
            .count();
        Metrics {
            sources: self.sources.len(),
            items: self.items.len(),
            events: self.events.len(),
            relations: self.relations.len(),
            verified_ratio: if self.items.is_empty() {
                0.0
            } else {
                verified as f64 / self.items.len() as f64
            },
            avg_reliability: avg_rel,
            primary_ratio: if self.sources.is_empty() {
                0.0
            } else {
                primary as f64 / self.sources.len() as f64
            },
            dangling_refs: dangling,
        }
    }

    /// Claims per status, always in `STATUSES` order, so the summary chips keep
    /// the same position from report to report and can be compared at a glance.
    pub fn status_counts(&self) -> Vec<(&'static str, usize)> {
        STATUSES
            .iter()
            .map(|st| (*st, self.items.iter().filter(|i| i.status == *st).count()))
            .collect()
    }

    /// How many claims cite each source id: the load-bearing sources first.
    pub fn citation_counts(&self) -> std::collections::BTreeMap<String, usize> {
        let mut map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for item in &self.items {
            for sid in &item.source_ids {
                *map.entry(sid.clone()).or_insert(0) += 1;
            }
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report {
            name: "t".into(),
            title: "T".into(),
            sources: vec![
                Source { url: "https://www.reuters.com/a".into(), ..Default::default() },
                Source { url: "https://x.com/b".into(), kind: "social".into(), ..Default::default() },
                Source { url: "https://www.reuters.com/a".into(), ..Default::default() },
            ],
            items: vec![
                Item { claim: "c1".into(), source_ids: vec!["1".into(), "s2".into()], ..Default::default() },
                Item { claim: "c2".into(), status: "refuted".into(), ..Default::default() },
            ],
            events: vec![
                Event { label: "A".into(), ..Default::default() },
                Event { label: "B".into(), ..Default::default() },
            ],
            relations: vec![
                Relation { from: "A".into(), to: "B".into(), kind: "caused".into(), ..Default::default() },
                Relation { from: "A".into(), to: "ghost".into(), ..Default::default() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn normalize_dedupes_sources_and_resolves_refs() {
        let mut r = sample();
        r.normalize();
        assert_eq!(r.sources.len(), 2, "duplicate URL must collapse");
        assert_eq!(r.items[0].source_ids, vec!["s1".to_string(), "s2".to_string()]);
        assert_eq!(r.items[0].status, "supported", "items with sources default to supported");
        assert_eq!(r.items[1].status, "refuted");
        assert!(r.items[1].confidence < 20);
    }

    #[test]
    fn relation_kinds_are_normalized_and_ghosts_dropped() {
        let mut r = sample();
        r.normalize();
        assert_eq!(r.relations.len(), 1);
        assert_eq!(r.relations[0].kind, "causes");
        assert_eq!(r.relations[0].strength, 2);
    }

    #[test]
    fn reliability_prior_prefers_institutions() {
        let mut r = Report {
            sources: vec![
                Source { url: "https://www.nasa.gov/x".into(), ..Default::default() },
                Source { url: "https://x.com/y".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        r.normalize();
        assert!(r.sources[0].reliability > r.sources[1].reliability);
        assert!(r.sources[0].id.starts_with('s'));
    }

    #[test]
    fn status_counts_and_citations_power_the_review_views() {
        let mut r = sample();
        r.normalize();
        let counts = r.status_counts();
        assert_eq!(counts.len(), 5, "all five review states must always be present");
        assert_eq!(
            counts.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec!["verified", "supported", "unverified", "disputed", "refuted"]
        );
        let supported = counts.iter().find(|(s, _)| *s == "supported").unwrap().1;
        assert_eq!(supported, 1, "item i1 has sources and defaults to supported");
        let refuted = counts.iter().find(|(s, _)| *s == "refuted").unwrap().1;
        assert_eq!(refuted, 1);

        let citations = r.citation_counts();
        assert_eq!(citations.get("s1").copied(), Some(1), "s1 is cited by the surviving claim");
        assert_eq!(citations.get("s2").copied(), Some(1), "s2 is cited by the same claim");
        assert!(citations.get("s3").is_none(), "the dropped duplicate must not be cited");
    }

    #[test]
    fn metrics_reported() {
        let mut r = sample();
        r.normalize();
        let m = r.metrics();
        assert_eq!(m.sources, 2);
        assert_eq!(m.items, 2);
        assert!(m.verified_ratio > 0.4 && m.verified_ratio < 0.6);
        assert_eq!(m.dangling_refs, 0);
    }
}
