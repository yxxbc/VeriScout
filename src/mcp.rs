//! MCP (Model Context Protocol) stdio server: the whole project as a tool.
//!
//! `ves mcp` speaks JSON-RPC 2.0 over stdin/stdout using the MCP stdio
//! transport (one message per line) and exposes:
//!   * the six sealed tools, unchanged and still behind every security gate;
//!   * `research`, which runs a complete investigation through the built-in
//!     agent loop and returns the answer plus report paths.
//!
//! stdout carries protocol traffic only; all logging goes to stderr.

use crate::agent::Agent;
use crate::config::{Config, VERSION};
use crate::tools::{self, Ctx};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Protocol revision advertised when the client does not request one.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub fn serve(cfg: Config) -> Result<()> {
    let mut ctx = Ctx::new(cfg);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                write_message(&mut out, &rpc_error(Value::Null, -32700, &format!("parse error: {e}")))?;
                continue;
            }
        };
        if let Some(response) = handle_message(&mut ctx, &msg) {
            write_message(&mut out, &response)?;
        }
    }
    Ok(())
}

fn write_message(out: &mut impl Write, message: &Value) -> Result<()> {
    serde_json::to_writer(&mut *out, message)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Handle one JSON-RPC message. Returns the response for requests, `None` for
/// notifications (messages without an id).
pub fn handle_message(ctx: &mut Ctx, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let id = id?;
            // Echo the client's revision when it sends one; MCP clients expect
            // the server to answer with a revision both sides can speak.
            let requested = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
            let version = if requested.is_empty() { PROTOCOL_VERSION } else { requested };
            Some(rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "VeriScout", "version": VERSION },
                    "instructions": "Web research with source verification. Use the six low-level tools to search, fetch and verify yourself, or call `research` for a full investigation (multi-engine search, primary-source checks, graded claims, offline HTML report)."
                }),
            ))
        }
        "notifications/initialized" | "notifications/cancelled" | "notifications/roots/list_changed" => None,
        "ping" => Some(rpc_result(id?, json!({}))),
        "tools/list" => Some(rpc_result(id?, json!({ "tools": tool_list() }))),
        "tools/call" => {
            let id = id?;
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Some(rpc_result(id, call_tool(ctx, name, &args)))
        }
        _ => match id {
            Some(id) => Some(rpc_error(id, -32601, &format!("method not found: {method}"))),
            None => None,
        },
    }
}

/// The six sealed tools plus one high-level research entry point.
fn tool_list() -> Vec<Value> {
    let mut tools: Vec<Value> = tools::registry()
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.parameters,
            })
        })
        .collect();
    tools.push(json!({
        "name": "research",
        "description": "Run a full VeriScout investigation for a question: multi-engine search, primary-source fetching, cross-verification with graded claims (verified/supported/disputed/refuted) and an offline HTML report. Slower than the low-level tools because it drives this project's own model loop, so a configured model/API key is required. Returns the final answer plus report paths.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The research question." },
                "max_steps": { "type": "integer", "description": "Optional step budget for this run (default from config)." }
            },
            "required": ["query"]
        }
    }));
    tools
}

fn call_tool(ctx: &mut Ctx, name: &str, args: &Value) -> Value {
    let outcome = if name == "research" {
        run_research(ctx, args)
    } else {
        tools::dispatch(ctx, name, args)
    };
    match outcome {
        Ok(value) => json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            }],
            "isError": false
        }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("error: {e}") }],
            "isError": true
        }),
    }
}

/// Full agent run behind the `research` tool.
fn run_research(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = tools::Args::new(args);
    let query = a.req_str("query")?;
    let mut cfg = ctx.cfg.clone();
    if let Some(steps) = args.get("max_steps").and_then(|v| v.as_u64()) {
        cfg.agent.max_steps = steps.clamp(1, 60) as u32;
    }
    let mut agent = Agent::new(cfg, false, false)?;
    let outcome = agent.run(&query)?;
    // Same contract as the CLI: the artifact carries the closing answer and the
    // tool log, so callers that only look at the report lose nothing.
    tools::html_tools::finalize(&mut agent.ctx, &outcome.new_reports, &outcome.answer, outcome.steps, outcome.elapsed_ms);
    let dir = agent.ctx.cfg.viz_dir();
    let reports: Vec<String> = outcome
        .new_reports
        .iter()
        .map(|name| dir.join(format!("{name}.html")).display().to_string())
        .collect();
    Ok(json!({
        "answer": outcome.answer,
        "reports": reports,
        "steps": outcome.steps,
        "tool_calls": outcome.tool_calls,
        "stopped_reason": outcome.stopped_reason,
        "usage": {
            "prompt_tokens": outcome.usage.prompt_tokens,
            "completion_tokens": outcome.usage.completion_tokens,
            "total_tokens": outcome.usage.total_tokens
        },
        "elapsed_ms": outcome.elapsed_ms
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx::new(Config::default())
    }

    #[test]
    fn initialize_echoes_the_requested_protocol_version() {
        let mut c = ctx();
        let resp = handle_message(
            &mut c,
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2024-11-05", "capabilities": {},
                            "clientInfo": { "name": "x", "version": "0" } }
            }),
        )
        .unwrap();
        assert_eq!(resp["jsonrpc"], json!("2.0"));
        assert_eq!(resp["id"], json!(1));
        assert_eq!(resp["result"]["protocolVersion"], json!("2024-11-05"));
        assert!(resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(resp["result"]["serverInfo"]["name"], json!("VeriScout"));
    }

    #[test]
    fn tools_list_covers_the_sealed_surface_plus_research() {
        let mut c = ctx();
        let resp = handle_message(&mut c, &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in ["web_search", "curl_tool", "scripts_tool", "create_html", "edit_html", "delete_html", "research"] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], json!("object"), "{} needs an object schema", tool["name"]);
        }
    }

    #[test]
    fn tool_errors_are_reported_as_mcp_errors_not_crashes() {
        let mut c = ctx();
        let resp = handle_message(
            &mut c,
            &json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "delete_html", "arguments": { "name": "definitely-not-here" } }
            }),
        )
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("error"));
    }

    #[test]
    fn notifications_are_silent_and_unknown_methods_error() {
        let mut c = ctx();
        assert!(handle_message(&mut c, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let resp = handle_message(&mut c, &json!({ "jsonrpc": "2.0", "id": 9, "method": "bogus" })).unwrap();
        assert_eq!(resp["error"]["code"], json!(-32601));
    }
}
