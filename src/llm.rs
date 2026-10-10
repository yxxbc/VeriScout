//! Minimal OpenAI-compatible chat client with tool calling.
//!
//! Works with DeepSeek, OpenAI, and any local server that speaks the same dialect
//! (Ollama's /v1, vLLM, LM Studio, ...). If a provider rejects the `tools` payload we
//! fall back to a fenced-JSON text protocol so the agent still functions.

use crate::config::Config;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: Some(content.into()), ..Default::default() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: Some(content.into()), ..Default::default() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: Some(content.into()), ..Default::default() }
    }
    pub fn tool(name: &str, call_id: &str, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_call_id: Some(call_id.to_string()),
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    pub fn text(&self) -> String {
        self.content.clone().unwrap_or_default()
    }

    /// Rough transcript weight, used for context budgeting.
    pub fn weight(&self) -> usize {
        self.text().chars().count()
            + self
                .tool_calls
                .iter()
                .map(|c| c.function.arguments.chars().count() + c.function.name.len())
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FunctionCall {
    pub name: String,
    /// JSON encoded string, as the API returns it.
    pub arguments: String,
}

impl ToolCall {
    pub fn arguments_json(&self) -> Value {
        if self.function.arguments.trim().is_empty() {
            return json!({});
        }
        serde_json::from_str(&self.function.arguments).unwrap_or_else(|_| json!({ "_raw": self.function.arguments }))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl Usage {
    fn add(&mut self, other: &Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
    }
}

#[derive(Debug, Clone)]
pub struct ChatOutcome {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub model: String,
    pub finish_reason: String,
}

#[derive(Debug)]
pub enum ChatError {
    Transport(String),
    Status(u16, String),
    Parse(String),
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatError::Transport(m) => write!(f, "network error talking to the model: {m}"),
            ChatError::Status(code, body) => write!(f, "model API returned HTTP {code}: {}", clip(body)),
            ChatError::Parse(m) => write!(f, "could not parse the model response: {m}"),
        }
    }
}

impl std::error::Error for ChatError {}

fn clip(s: &str) -> String {
    let cleaned = s.trim();
    if cleaned.chars().count() <= 400 {
        return cleaned.to_string();
    }
    cleaned.chars().take(400).collect::<String>() + "…"
}

pub struct LlmClient {
    pub base_url: String,
    pub model: String,
    pub temperature: f64,
    pub max_tokens: u32,
    key: Option<String>,
    agent: ureq::Agent,
    pub calls: u32,
    pub usage: Usage,
}

impl LlmClient {
    pub fn new(cfg: &Config) -> Result<Self> {
        let key = cfg.resolve_api_key().map(|(k, _)| k);
        if key.is_none() && !cfg.llm.provider.eq_ignore_ascii_case("ollama") {
            return Err(anyhow!(
                "no API key found for provider '{}'. Set the {} environment variable or add \"api_key\" to the config file (see ‘ves config show’).",
                cfg.llm.provider,
                cfg.llm.api_key_env
            ));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout(Duration::from_secs(cfg.llm.timeout_secs.max(30)))
            .build();
        Ok(Self {
            base_url: cfg.llm.base_url.trim_end_matches('/').to_string(),
            model: cfg.llm.model.clone(),
            temperature: cfg.llm.temperature,
            max_tokens: cfg.llm.max_tokens,
            key,
            agent,
            calls: 0,
            usage: Usage::default(),
        })
    }

    pub fn describe(&self) -> String {
        format!("{} @ {}", self.model, self.base_url)
    }

    fn endpoint(&self) -> String {
        if self.base_url.ends_with("/chat/completions") {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }

    /// One completion round. `tools` may be empty for a plain answer.
    pub fn chat(&mut self, messages: &[Message], tools: &Value) -> std::result::Result<ChatOutcome, ChatError> {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "stream": false,
        });
        if tools.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
            body["tools"] = tools.clone();
            body["tool_choice"] = json!("auto");
        }
        let mut req = self
            .agent
            .post(&self.endpoint())
            .set("Content-Type", "application/json")
            .set("Accept", "application/json");
        if let Some(key) = &self.key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        let payload = serde_json::to_string(&body).map_err(|e| ChatError::Parse(e.to_string()))?;
        let resp = match req.send_string(&payload) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(ChatError::Status(code, text));
            }
            Err(ureq::Error::Transport(t)) => return Err(ChatError::Transport(t.to_string())),
        };
        let text = resp.into_string().map_err(|e| ChatError::Transport(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| ChatError::Parse(format!("{e}: {}", clip(&text))))?;
        self.calls += 1;
        let usage = Usage {
            prompt_tokens: v.pointer("/usage/prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            completion_tokens: v.pointer("/usage/completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            total_tokens: v.pointer("/usage/total_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        };
        self.usage.add(&usage);
        let message = v
            .pointer("/choices/0/message")
            .cloned()
            .ok_or_else(|| ChatError::Parse(format!("no choices[0].message in: {}", clip(&text))))?;
        let content = message
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let mut tool_calls: Vec<ToolCall> = message
            .get("tool_calls")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or_default();
        for (i, call) in tool_calls.iter_mut().enumerate() {
            if call.id.trim().is_empty() {
                call.id = format!("call_{i}");
            }
            if call.kind.trim().is_empty() {
                call.kind = "function".into();
            }
        }
        if tool_calls.is_empty() {
            tool_calls = parse_text_protocol(&content);
        }
        Ok(ChatOutcome {
            content,
            tool_calls,
            usage,
            model: v.get("model").and_then(|m| m.as_str()).unwrap_or(&self.model).to_string(),
            finish_reason: v
                .pointer("/choices/0/finish_reason")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

/// Providers without native tool calling can still drive the agent by emitting a
/// fenced json block shaped like {"tool": "web_search", "args": {...}}.
pub fn parse_text_protocol(content: &str) -> Vec<ToolCall> {
    const FENCE: &str = "\u{60}\u{60}\u{60}";
    let mut out = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find(FENCE) {
        let after = &rest[start + FENCE.len()..];
        let Some(end) = after.find(FENCE) else { break };
        let block = &after[..end];
        let body = block.trim();
        let body = body.strip_prefix("json").unwrap_or(body).trim();
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            let name = v
                .get("tool")
                .or_else(|| v.get("name"))
                .or_else(|| v.get("function"))
                .and_then(|x| x.as_str());
            let args = v
                .get("args")
                .or_else(|| v.get("arguments"))
                .or_else(|| v.get("parameters"))
                .cloned()
                .unwrap_or(json!({}));
            if let Some(name) = name {
                out.push(ToolCall {
                    id: format!("textcall_{}", out.len()),
                    kind: "function".into(),
                    function: FunctionCall {
                        name: name.to_string(),
                        arguments: args.to_string(),
                    },
                });
            }
        }
        rest = &after[end + FENCE.len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_serialization_matches_openai_shape() {
        let m = Message::tool("web_search", "call_1", "{}");
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "call_1");
        let a = Message::assistant("hi");
        assert!(serde_json::to_value(&a).unwrap().get("tool_calls").is_none());
    }

    #[test]
    fn tool_arguments_tolerate_garbage() {
        let call = ToolCall {
            id: "1".into(),
            kind: "function".into(),
            function: FunctionCall { name: "x".into(), arguments: "not json".into() },
        };
        assert!(call.arguments_json().get("_raw").is_some());
    }

    #[test]
    fn text_protocol_is_parsed() {
        let content = "Let me search.\n\u{60}\u{60}\u{60}json\n{\"tool\": \"web_search\", \"args\": {\"query\": \"rust\"}}\n\u{60}\u{60}\u{60}\n";
        let calls = parse_text_protocol(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "web_search");
        assert_eq!(calls[0].arguments_json()["query"], "rust");
    }

    #[test]
    fn missing_key_is_reported_clearly() {
        let mut cfg = Config::default();
        cfg.llm.api_key.clear();
        cfg.llm.api_key_env = "VES_DEFINITELY_UNSET_KEY".into();
        cfg.llm.credential_files.clear();
        assert!(LlmClient::new(&cfg).is_err());
    }

    #[test]
    fn endpoint_appends_path_once() {
        let mut cfg = Config::default();
        cfg.llm.api_key = "sk-test".into();
        let client = LlmClient::new(&cfg).unwrap();
        assert!(client.endpoint().ends_with("/v1/chat/completions"));
        let mut cfg2 = cfg.clone();
        cfg2.llm.base_url = "https://x.example/v1/chat/completions".into();
        assert_eq!(LlmClient::new(&cfg2).unwrap().endpoint(), "https://x.example/v1/chat/completions");
    }
}
