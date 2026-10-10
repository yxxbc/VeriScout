//! Domain blacklist / allowlist engine shared by every network-facing tool.

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
enum Rule {
    /// `example.com` — matches the apex domain and every subdomain.
    Domain(String),
    /// `*.example.com` — matches subdomains only.
    Subdomain(String),
    /// `re:^ads?\\.` — raw regular expression against the host.
    Pattern(Regex),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Blacklist {
    /// Domains/patterns that are refused. Matching is on the registrable host.
    pub blocked: Vec<String>,
    /// Domains/patterns that are always allowed, even if a broad block rule matches.
    pub allowed: Vec<String>,
    /// Refuse private, loopback and link-local addresses (SSRF guard).
    pub block_private_addresses: bool,
}

#[derive(Debug, Clone)]
pub struct Matcher {
    rules: Vec<Rule>,
    allow: Vec<Rule>,
    pub block_private_addresses: bool,
}

impl Matcher {
    pub fn new(blacklist: &Blacklist) -> Self {
        Self {
            rules: compile(&blacklist.blocked),
            allow: compile(&blacklist.allowed),
            block_private_addresses: blacklist.block_private_addresses,
        }
    }

    /// `Some(reason)` when the host must not be contacted.
    pub fn check_host(&self, host: &str) -> Option<String> {
        let host = normalize_host(host);
        if host.is_empty() {
            return Some("empty host".into());
        }
        if self.matches(&self.allow, &host).is_some() {
            return None;
        }
        if let Some(rule) = self.matches(&self.rules, &host) {
            return Some(format!("host '{}' matches block rule '{}'", host, rule));
        }
        if self.block_private_addresses && is_private_host(&host) {
            return Some(format!("host '{}' resolves to a private/loopback address space", host));
        }
        None
    }

    fn matches(&self, rules: &[Rule], host: &str) -> Option<String> {
        for rule in rules {
            match rule {
                Rule::Domain(d) => {
                    if host == *d || host.ends_with(&format!(".{d}")) {
                        return Some(d.clone());
                    }
                }
                Rule::Subdomain(d) => {
                    if host.ends_with(&format!(".{d}")) {
                        return Some(format!("*.{d}"));
                    }
                }
                Rule::Pattern(re) => {
                    if re.is_match(host) {
                        return Some(format!("re:{}", re.as_str()));
                    }
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn is_blocked(&self, host: &str) -> bool {
        self.check_host(host).is_some()
    }

    /// Count of active block rules (reported in visualizations as provenance).
    #[allow(dead_code)]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

fn compile(patterns: &[String]) -> Vec<Rule> {
    let mut out = Vec::new();
    for raw in patterns {
        let p = raw.trim();
        if p.is_empty() || p.starts_with('#') {
            continue;
        }
        if let Some(rest) = p.strip_prefix("re:") {
            if let Ok(re) = Regex::new(&format!("(?i){rest}")) {
                out.push(Rule::Pattern(re));
            }
            continue;
        }
        let lower = p.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("*.") {
            out.push(Rule::Subdomain(rest.trim_end_matches('.').to_string()));
        } else {
            out.push(Rule::Domain(lower.trim_end_matches('.').to_string()));
        }
    }
    out
}

pub fn normalize_host(host: &str) -> String {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    h.strip_prefix("www.").unwrap_or(&h).to_string()
}

/// Domain-level trust prior used when scoring search results (0-100).
pub fn trust_prior(host: &str) -> i32 {
    let h = normalize_host(host);
    if h.is_empty() {
        return 40;
    }
    if h.ends_with(".gov") || h.ends_with(".gov.cn") || h.ends_with(".gov.uk") || h.ends_with(".edu") || h.ends_with(".edu.cn") || h.ends_with(".ac.uk") {
        return 92;
    }
    const HIGH: [&str; 26] = [
        "reuters.com", "apnews.com", "bbc.com", "bbc.co.uk", "nature.com", "science.org", "who.int",
        "un.org", "nasa.gov", "nih.gov", "cdc.gov", "europa.eu", "imf.org", "worldbank.org",
        "xinhuanet.com", "news.cn", "people.com.cn", "gov.cn", "caixin.com", "thepaper.cn",
        "wikipedia.org", "britannica.com", "ourworldindata.org", "arxiv.org", "doi.org", "pubmed.ncbi.nlm.nih.gov",
    ];
    if HIGH.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))) {
        return 88;
    }
    const MEDIUM: [&str; 22] = [
        "nytimes.com", "wsj.com", "ft.com", "bloomberg.com", "economist.com", "theguardian.com",
        "washingtonpost.com", "cnbc.com", "forbes.com", "techcrunch.com", "theverge.com",
        "arstechnica.com", "github.com", "stackoverflow.com", "medium.com", "zhihu.com",
        "36kr.com", "jiemian.com", "yicai.com", "cls.cn", "solidot.org", "infoq.com",
    ];
    if MEDIUM.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))) {
        return 74;
    }
    const LOW: [&str; 14] = [
        "facebook.com", "x.com", "twitter.com", "tiktok.com", "weibo.com", "pinterest.com",
        "quora.com", "answers.com", "blogspot.com", "wordpress.com", "substack.com",
        "reddit.com", "tieba.baidu.com", "csdn.net",
    ];
    if LOW.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))) {
        return 52;
    }
    if h.ends_with(".ai") || h.ends_with(".io") {
        return 60;
    }
    62
}

pub fn is_private_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']);
    if h.eq_ignore_ascii_case("localhost") || h.ends_with(".localhost") || h.ends_with(".local") || h == "::1" {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_broadcast() || v4.is_unspecified()
                    || v4.octets()[0] == 0
                    || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])) // CGNAT
                    || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Matcher {
        Matcher::new(&Blacklist {
            blocked: vec!["spam.example".into(), "*.tracker.example".into(), "re:^ads?[0-9]*\\.".into()],
            allowed: vec!["good.spam.example".into()],
            block_private_addresses: true,
        })
    }

    #[test]
    fn blocks_domains_and_subdomains() {
        let m = m();
        assert!(m.is_blocked("spam.example"));
        assert!(m.is_blocked("news.spam.example"));
        assert!(m.is_blocked("a.tracker.example"));
        assert!(!m.is_blocked("tracker.example"), "wildcard must not match the apex");
        assert!(m.is_blocked("ad7.somewhere.com"));
    }

    #[test]
    fn allowlist_wins() {
        assert!(!m().is_blocked("good.spam.example"));
    }

    #[test]
    fn private_addresses_rejected() {
        let m = m();
        assert!(m.is_blocked("127.0.0.1"));
        assert!(m.is_blocked("localhost"));
        assert!(m.is_blocked("192.168.1.10"));
        assert!(m.is_blocked("169.254.169.254"));
        assert!(!m.is_blocked("1.1.1.1"));
    }

    #[test]
    fn trust_prior_ranks_institutions_highest() {
        assert!(trust_prior("www.nasa.gov") > trust_prior("someblog.com"));
        assert!(trust_prior("x.com") < trust_prior("reuters.com"));
    }
}
