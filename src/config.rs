//! Runtime configuration: defaults, file discovery, credential resolution.

use crate::blacklist::Blacklist;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub llm: Llm,
    pub net: Net,
    pub search: Search,
    pub curl: Curl,
    pub scripts: Scripts,
    pub viz: Viz,
    pub agent: Agent,
}

/// Network behaviour shared by every outbound request (search + curl).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Net {
    /// Extra attempts after a transport failure, or after 429/5xx on GET/HEAD.
    pub retries: u32,
    /// Base backoff in milliseconds; grows linearly with the attempt number.
    pub backoff_ms: u64,
}

impl Default for Net {
    fn default() -> Self {
        Self { retries: 3, backoff_ms: 300 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Llm {
    /// deepseek | openai | ollama | custom
    pub provider: String,
    pub base_url: String,
    pub model: String,
    /// Name of the environment variable that holds the key.
    pub api_key_env: String,
    /// Inline key (discouraged; takes precedence when set).
    pub api_key: String,
    /// Extra files to search for KEY=value / KEY: value credential lines.
    pub credential_files: Vec<String>,
    pub temperature: f64,
    pub max_tokens: u32,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Search {
    /// General engines: bing, duckduckgo, wikipedia, searx, mojeek.
    pub engines: Vec<String>,
    /// Vertical engines: googlenews, crossref, openalex, arxiv, hackernews, github, stackexchange.
    pub vertical_engines: Vec<String>,
    /// SearxNG instances tried in order (JSON API first, HTML fallback).
    pub searx_instances: Vec<String>,
    pub results_per_engine: usize,
    pub max_results: usize,
    pub timeout_secs: u64,
    pub user_agent: String,
    /// Key-based engines are used only when the matching key is present.
    pub tavily_api_key_env: String,
    pub serper_api_key_env: String,
    pub brave_api_key_env: String,
    /// Custom search endpoint (anysearch-style aggregator), empty = disabled.
    pub custom_endpoint: String,
    pub custom_api_key_env: String,
    /// Directory of the local AnySearch CLI skill (contains scripts/anysearch_cli.{js,py,sh}).
    pub anysearch_skill_dir: String,
    /// Runtime used to launch the CLI: node | python3 | bash. Empty = auto-detect.
    pub anysearch_runtime: String,
    pub anysearch_timeout_secs: u64,
    /// Use the CLI's `extract` command for pages normal fetching cannot read (JS-heavy sites).
    pub anysearch_extract: bool,
    /// Fetch the top N result pages and extract text so claims can be checked against primary text.
    pub deep_fetch_pages: usize,
    pub max_body_bytes: usize,
    pub blacklist: Blacklist,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Curl {
    /// native (built-in HTTP client, no shell) | binary (system curl with a strict argument allowlist)
    pub engine: String,
    /// Path or name of the curl executable used by the binary engine.
    pub binary_path: String,
    pub timeout_secs: u64,
    pub max_bytes: usize,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub user_agent: String,
    /// Directory (relative to the workspace) that save=true responses are written into.
    pub save_dir: String,
    /// Allow requests to private/loopback addresses (off by default: SSRF guard).
    pub allow_private_addresses: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Scripts {
    pub python: String,
    pub timeout_secs: u64,
    /// Wall-clock cap enforced inside the interpreter as well.
    pub cpu_seconds: u64,
    pub max_memory_mb: u64,
    pub max_output_bytes: usize,
    /// Modules the sandboxed script may import.
    pub allowed_modules: Vec<String>,
    /// Hosts the script may contact (empty = any host, minus blacklist).
    pub allow_hosts: Vec<String>,
    pub deny_hosts: Vec<String>,
    pub max_fetch_bytes: usize,
    pub network_enabled: bool,
    pub keep_temp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Viz {
    /// Output directory for generated HTML (relative to the workspace).
    pub dir: String,
    /// dark | light | auto
    pub theme: String,
    /// Also keep the JSON model next to each report (required by edit_html).
    pub keep_model: bool,
    pub open_after_create: bool,
    /// Language used in the generated HTML chrome.
    pub locale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Agent {
    pub max_steps: u32,
    /// Hard cap on tool calls in a single run.
    pub max_tool_calls: u32,
    /// Tool results are clipped to this many characters before entering the transcript.
    pub tool_output_chars: usize,
    /// Once the transcript exceeds this, older tool results are replaced by stubs.
    pub context_budget_chars: usize,
    pub system_prompt_extra: String,
    /// Output language written into the system prompt ("zh" renders as 中文; other values are used as-is).
    pub language: String,
    /// Answer style: "plain" appends a plain-language clause to the system prompt; any other value leaves it out.
    pub style: String,
    /// Print each tool call/result to stderr.
    pub trace: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            llm: Llm::default(),
            net: Net::default(),
            search: Search::default(),
            curl: Curl::default(),
            scripts: Scripts::default(),
            viz: Viz::default(),
            agent: Agent::default(),
        }
    }
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            provider: "deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            api_key_env: "DEEPSEEK_API_KEY".into(),
            api_key: String::new(),
            credential_files: vec![
                "~/.dsh/.credentials.yaml".into(),
                "~/.ves/credentials.yaml".into(),
            ],
            temperature: 0.2,
            max_tokens: 8192,
            timeout_secs: 180,
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            engines: vec!["anysearch".into(), "bing".into(), "wikipedia".into(), "duckduckgo".into(), "searx".into()],
            vertical_engines: vec![
                "googlenews".into(),
                "crossref".into(),
                "openalex".into(),
                "arxiv".into(),
                "hackernews".into(),
                "github".into(),
                "stackexchange".into(),
            ],
            searx_instances: vec![
                "https://searx.be".into(),
                "https://search.inetol.net".into(),
                "https://baresearch.org".into(),
                "https://opnxng.com".into(),
                "https://priv.au".into(),
            ],
            results_per_engine: 8,
            max_results: 30,
            timeout_secs: 20,
            user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36".into(),
            tavily_api_key_env: "TAVILY_API_KEY".into(),
            serper_api_key_env: "SERPER_API_KEY".into(),
            brave_api_key_env: "BRAVE_API_KEY".into(),
            custom_endpoint: String::new(),
            custom_api_key_env: "ANYSEARCH_API_KEY".into(),
            anysearch_skill_dir: "~/.agents/skills/anysearch".into(),
            anysearch_runtime: String::new(),
            anysearch_timeout_secs: 45,
            anysearch_extract: true,
            deep_fetch_pages: 0,
            max_body_bytes: 2_000_000,
            blacklist: Blacklist {
                blocked: default_blocklist(),
                // a few useful exceptions to broad rules (Baidu Baike survives the baidu.com block)
                allowed: vec!["baike.baidu.com".into()],
                block_private_addresses: true,
            },
        }
    }
}

impl Default for Curl {
    fn default() -> Self {
        Self {
            engine: "native".into(),
            binary_path: "curl".into(),
            timeout_secs: 30,
            max_bytes: 400_000,
            follow_redirects: true,
            max_redirects: 5,
            user_agent: "ves/0.1 (+https://github.com/local/veriscout)".into(),
            save_dir: "raw".into(),
            allow_private_addresses: false,
        }
    }
}

impl Default for Scripts {
    fn default() -> Self {
        Self {
            // Windows ships `python` on PATH; macOS/Linux usually only have `python3`.
            python: if cfg!(windows) { "python".to_string() } else { "python3".to_string() },
            timeout_secs: 45,
            cpu_seconds: 40,
            max_memory_mb: 512,
            max_output_bytes: 200_000,
            allowed_modules: vec![
                "json", "re", "math", "statistics", "datetime", "time", "random", "string",
                "itertools", "functools", "collections", "collections.abc", "typing", "dataclasses",
                "decimal", "fractions", "hashlib", "hmac", "base64", "binascii", "textwrap",
                "unicodedata", "difflib", "html", "csv", "urllib.parse", "urllib.error",
                "urllib.request", "http.client", "email.utils", "uuid", "enum", "operator", "copy",
                "contextlib", "warnings",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            allow_hosts: Vec::new(),
            deny_hosts: vec!["localhost".into(), "127.0.0.1".into()],
            max_fetch_bytes: 1_000_000,
            network_enabled: true,
            keep_temp: false,
        }
    }
}

impl Default for Viz {
    fn default() -> Self {
        Self {
            dir: "visualizations".into(),
            theme: "dark".into(),
            keep_model: true,
            open_after_create: false,
            locale: "zh".into(),
        }
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            max_steps: 18,
            max_tool_calls: 40,
            tool_output_chars: 12_000,
            context_budget_chars: 220_000,
            system_prompt_extra: String::new(),
            language: "zh".into(),
            style: "plain".into(),
            trace: false,
        }
    }
}

/// Default blocked domains: content farms, scrapers and known SEO noise.
fn default_blocklist() -> Vec<String> {
    [
        "pinterest.com",
        "quora.com",
        "answers.com",
        "ask.com",
        "ehow.com",
        "livestrong.com",
        "slideshare.net",
        "scribd.com",
        "coursehero.com",
        "chegg.com",
        "brainly.com",
        "search.yahoo.com",
        "baidu.com",
        "so.com",
        "sogou.com",
        "hao123.com",
        "toutiao.com",
        "ixigua.com",
        "dzen.ru",
        "w3schools.com",
        "javatpoint.com",
        "geeksforgeeks.org",
        "tutorialspoint.com",
        "programiz.com",
        "googleadservices.com",
        "doubleclick.net",
        "taboola.com",
        "outbrain.com",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Config {
    /// Resolve the config path: explicit flag > VES_CONFIG env > ./.ves/config.json > ~/.ves/config.json
    pub fn resolve_path(explicit: Option<&str>) -> (Option<PathBuf>, PathBuf) {
        if let Some(p) = explicit {
            let pb = expand_tilde(p);
            return (Some(pb.clone()), pb);
        }
        if let Ok(env_path) = std::env::var("VES_CONFIG") {
            if !env_path.trim().is_empty() {
                let pb = expand_tilde(&env_path);
                return (Some(pb.clone()), pb);
            }
        }
        let local = PathBuf::from(".ves/config.json");
        if local.exists() {
            return (Some(local.clone()), local);
        }
        let home = expand_tilde("~/.ves/config.json");
        if home.exists() {
            return (Some(home.clone()), home);
        }
        (None, local)
    }

    pub fn load(explicit: Option<&str>) -> Result<(Config, Option<PathBuf>)> {
        let (found, default_path) = Self::resolve_path(explicit);
        match found {
            Some(p) => {
                let raw = std::fs::read_to_string(&p)
                    .with_context(|| format!("cannot read config file {}", p.display()))?;
                let cfg: Config = serde_json::from_str(&raw)
                    .with_context(|| format!("invalid JSON in config file {}", p.display()))?;
                Ok((cfg, Some(p)))
            }
            None => Ok((Config::default(), Some(default_path))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("cannot write config {}", path.display()))?;
        Ok(())
    }

    /// Look for an API key: inline config > environment > credential files.
    pub fn resolve_api_key(&self) -> Option<(String, String)> {
        if !self.llm.api_key.trim().is_empty() {
            return Some((self.llm.api_key.trim().to_string(), "config file".into()));
        }
        let env_name = self.llm.api_key_env.trim();
        if !env_name.is_empty() {
            if let Ok(v) = std::env::var(env_name) {
                if !v.trim().is_empty() {
                    return Some((v.trim().to_string(), format!("env {}", env_name)));
                }
            }
        }
        if self.llm.provider == "deepseek" {
            if let Ok(v) = std::env::var("DEEPSEEK_API_KEY") {
                if !v.trim().is_empty() {
                    return Some((v.trim().to_string(), "env DEEPSEEK_API_KEY".into()));
                }
            }
        }
        if !env_name.is_empty() {
            for file in &self.llm.credential_files {
                let p = expand_tilde(file);
                if let Ok(text) = std::fs::read_to_string(&p) {
                    if let Some(v) = find_credential(&text, env_name) {
                        return Some((v, format!("{}: {}", p.display(), env_name)));
                    }
                }
            }
        }
        None
    }

    pub fn search_api_key(&self, env_name: &str) -> Option<String> {
        if env_name.trim().is_empty() {
            return None;
        }
        if let Ok(v) = std::env::var(env_name) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
        for file in &self.llm.credential_files {
            let p = expand_tilde(file);
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Some(v) = find_credential(&text, env_name) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Workspace root: where visualizations/, raw/ and .ves/ live.
    pub fn workspace_root() -> PathBuf {
        if let Ok(v) = std::env::var("VES_WORKSPACE") {
            if !v.trim().is_empty() {
                return expand_tilde(&v);
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    pub fn viz_dir(&self) -> PathBuf {
        let p = expand_tilde(&self.viz.dir);
        if p.is_absolute() {
            p
        } else {
            Self::workspace_root().join(p)
        }
    }

    pub fn raw_dir(&self) -> PathBuf {
        let p = expand_tilde(&self.curl.save_dir);
        if p.is_absolute() {
            p
        } else {
            Self::workspace_root().join(p)
        }
    }

    pub fn all_engines(&self) -> Vec<String> {
        let mut v = self.search.engines.clone();
        v.extend(self.search.vertical_engines.clone());
        v
    }
}

/// Pull KEY: value / KEY=value (also quoted) out of a credential file.
pub fn find_credential(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let (k, v) = if let Some(i) = line.find(':') {
            (&line[..i], &line[i + 1..])
        } else if let Some(i) = line.find('=') {
            (&line[..i], &line[i + 1..])
        } else {
            continue;
        };
        let k = k.trim().trim_matches('"').trim_matches('\'');
        if k == key {
            let val = v.trim().trim_matches('"').trim_matches('\'').trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// Everything worth showing in `ves config show`, with secrets masked.
pub fn redacted_summary(cfg: &Config) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("provider".into(), cfg.llm.provider.clone());
    m.insert("base_url".into(), cfg.llm.base_url.clone());
    m.insert("model".into(), cfg.llm.model.clone());
    m.insert(
        "api_key".into(),
        match cfg.resolve_api_key() {
            Some((k, src)) => {
                let head: String = k.chars().take(6).collect();
                format!("{}…({} chars) via {}", head, k.chars().count(), src)
            }
            None => "<missing>".into(),
        },
    );
    m.insert("search.engines".into(), cfg.search.engines.join(","));
    m.insert("search.verticals".into(), cfg.search.vertical_engines.join(","));
    m.insert("search.blocked_rules".into(), cfg.search.blacklist.blocked.len().to_string());
    m.insert("curl.engine".into(), cfg.curl.engine.clone());
    m.insert("scripts.python".into(), cfg.scripts.python.clone());
    m.insert("viz.dir".into(), cfg.viz_dir().display().to_string());
    m.insert("agent.max_steps".into(), cfg.agent.max_steps.to_string());
    m.insert("agent.style".into(), cfg.agent.style.clone());
    m.insert("agent.language".into(), cfg.agent.language.clone());
    m.insert("time.local".into(), crate::util::iso8601_now_local());
    m.insert("time.utc".into(), crate::util::iso8601_now());
    m.insert("workspace".into(), Config::workspace_root().display().to_string());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert_eq!(c.llm.provider, "deepseek");
        assert!(c.search.engines.contains(&"bing".to_string()));
        assert!(c.search.engines.contains(&"anysearch".to_string()));
        assert!(c.search.anysearch_extract);
        assert!(c.search.blacklist.block_private_addresses);
        assert_eq!(c.viz.dir, "visualizations");
        assert_eq!(c.agent.language, "zh");
        assert_eq!(c.agent.style, "plain");
    }

    #[test]
    fn missing_fields_use_defaults() {
        let partial: Config = serde_json::from_str(r#"{"llm":{"model":"deepseek-reasoner"}}"#).unwrap();
        assert_eq!(partial.llm.model, "deepseek-reasoner");
        assert_eq!(partial.llm.base_url, "https://api.deepseek.com/v1");
        assert_eq!(partial.agent.max_steps, 18);
        assert_eq!(partial.net.retries, 3, "network retries default must survive partial configs");
    }

    #[test]
    fn credential_parsing_handles_yaml_and_env_shapes() {
        let text = "version: 1\nrefs:\n  DEEPSEEK_API_KEY: sk-abc123\n  OTHER=\"x\"\n";
        assert_eq!(find_credential(text, "DEEPSEEK_API_KEY").as_deref(), Some("sk-abc123"));
        assert_eq!(find_credential(text, "OTHER").as_deref(), Some("x"));
        assert_eq!(find_credential(text, "NOPE"), None);
    }

    #[test]
    fn inline_key_beats_everything() {
        let mut c = Config::default();
        c.llm.api_key = "  sk-inline  ".into();
        let (k, src) = c.resolve_api_key().unwrap();
        assert_eq!(k, "sk-inline");
        assert_eq!(src, "config file");
    }

    #[test]
    fn blocked_rules_compile() {
        let c = Config::default();
        let m = crate::blacklist::Matcher::new(&c.search.blacklist);
        assert!(m.is_blocked("www.pinterest.com"));
        assert!(m.rule_count() > 10);
    }
}
