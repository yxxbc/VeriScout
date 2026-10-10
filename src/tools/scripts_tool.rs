//! scripts_tool - run a short Python script inside a network-only sandbox.
//!
//! The script is validated (AST, static) by the embedded bootstrap before execution,
//! then run with a restricted builtin set, a guarded import hook, a socket-level host
//! policy and rlimits. See assets/python/ves_sandbox.py for the enforcement details.

use super::{Args, Ctx};
use crate::config::Config;
use crate::util;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SANDBOX_PY: &str = include_str!("../../assets/python/ves_sandbox.py");

pub fn run(ctx: &mut Ctx, args: &Value) -> Result<Value> {
    let a = Args::new(args);
    let code = a.req_str("code")?;
    if code.len() > 200_000 {
        return Err(anyhow!("script too large ({} bytes, max 200000)", code.len()));
    }
    let timeout = a.u64("timeout_secs", ctx.cfg.scripts.timeout_secs).clamp(1, 120);
    let stdin_text = a.str("stdin");

    let python = ctx.cfg.scripts.python.trim().to_string();
    let dir = std::env::temp_dir().join(format!(
        "ves-script-{}-{}",
        std::process::id(),
        &util::short_hash(&code)[..8]
    ));
    std::fs::create_dir_all(&dir).map_err(|e| anyhow!("cannot create sandbox dir {}: {e}", dir.display()))?;

    let script_path = dir.join("script.py");
    let config_path = dir.join("sandbox.json");
    let runner_path = dir.join("ves_sandbox.py");
    std::fs::write(&script_path, &code)?;
    std::fs::write(&runner_path, SANDBOX_PY)?;
    std::fs::write(&config_path, serde_json::to_vec_pretty(&sandbox_config(&ctx.cfg))?)?;

    let mut cmd = Command::new(&python);
    cmd.arg(&runner_path)
        .arg(&script_path)
        .current_dir(&dir)
        // a clean environment: no API keys, no proxy credentials, no user config
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin")
        .env("HOME", &dir)
        .env("TMPDIR", &dir)
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONHASHSEED", "0")
        .env("VES_SANDBOX_CONFIG", &config_path)
        .stdin(if stdin_text.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(anyhow!(
                "cannot start '{}': {e}. Install Python 3 or set scripts.python in the config.",
                python
            ));
        }
    };

    let limit = ctx.cfg.scripts.max_output_bytes.max(4096);
    let out_pipe = child.stdout.take().expect("stdout piped");
    let err_pipe = child.stderr.take().expect("stderr piped");
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.take(limit as u64 + 8192).read_to_end(&mut buf);
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.take(limit as u64 + 8192).read_to_end(&mut buf);
        buf
    });
    if let Some(text) = &stdin_text {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(text.as_bytes());
        }
    }

    let deadline = started + Duration::from_secs(timeout);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(anyhow!("waiting for python failed: {e}")),
        }
    };
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    if !ctx.cfg.scripts.keep_temp {
        let _ = std::fs::remove_dir_all(&dir);
    }

    let stdout_text = String::from_utf8_lossy(&stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&stderr).to_string();
    let exit_code = status.as_ref().and_then(|s| s.code()).unwrap_or(-1);
    let duration_ms = started.elapsed().as_millis();
    let dir_note = if ctx.cfg.scripts.keep_temp {
        Some(dir.display().to_string())
    } else {
        None
    };

    let (stdout_text, out_trunc) = util::truncate_chars(&stdout_text, limit);
    let (stderr_text, err_trunc) = util::truncate_chars(&stderr_text, limit);

    // Policy rejection / timeout / crash are reported distinctly so the model can react.
    if exit_code == 3 {
        let reason = stderr_text
            .lines()
            .find_map(|l| l.strip_prefix("VES_REJECT: "))
            .unwrap_or(stderr_text.trim())
            .to_string();
        return Err(anyhow!(
            "script rejected by the network-only sandbox: {reason}\n             Allowed imports: {}. Filesystem, subprocess, socket and dynamic-execution APIs are never available.\n             Available helpers: fetch(url, method=..., headers=..., data=..., timeout=..., max_bytes=...), fetch_json(url, ...), html_to_text(html), strip_tags(s), truncate(s, n), emit(obj), now(), json, re, math.",
            ctx.cfg.scripts.allowed_modules.join(", ")
        ));
    }
    if timed_out || exit_code == 5 {
        return Ok(json!({
            "ok": false,
            "timed_out": true,
            "exit_code": exit_code,
            "stdout": stdout_text,
            "stderr": stderr_text,
            "error": format!("script exceeded the {timeout}s limit and was killed; make it cheaper or split the work"),
            "duration_ms": duration_ms,
        }));
    }

    let ok = exit_code == 0;
    let mut out = json!({
        "ok": ok,
        "exit_code": exit_code,
        "duration_ms": duration_ms,
        "stdout": stdout_text,
        "stderr": stderr_text,
        "stdout_truncated": out_trunc,
        "stderr_truncated": err_trunc,
    });
    if !ok {
        let err_line = stderr_text
            .lines()
            .rev()
            .find(|l| l.starts_with("VES_ERROR:"))
            .map(|l| l.trim_start_matches("VES_ERROR:").trim().to_string())
            .unwrap_or_else(|| "script exited with a non-zero status".to_string());
        out["error"] = json!(err_line);
    }
    if let Some(d) = dir_note {
        out["sandbox_dir"] = json!(d);
    }
    Ok(out)
}

fn sandbox_config(cfg: &Config) -> Value {
    json!({
        "allowed_modules": cfg.scripts.allowed_modules,
        "allow_hosts": cfg.scripts.allow_hosts,
        "deny_hosts": cfg.scripts.deny_hosts,
        "max_fetch_bytes": cfg.scripts.max_fetch_bytes,
        "network_enabled": cfg.scripts.network_enabled,
        "cpu_seconds": cfg.scripts.timeout_secs,
        "max_memory_mb": cfg.scripts.max_memory_mb,
        "max_source_bytes": 200_000,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx::new(Config::default())
    }

    #[test]
    fn rejects_filesystem_and_process_code() {
        let mut c = ctx();
        for bad in [
            "import os\nprint(os.listdir('.'))",
            "import subprocess\nsubprocess.run(['ls'])",
            "print(open('/etc/passwd').read())",
            "exec('print(1)')",
            "print(().__class__)",
            "print(getattr(1, '__class__'))",
            "import socket\ns = socket.socket()",
            "from os import system",
        ] {
            let err = run(&mut c, &json!({ "code": bad })).unwrap_err().to_string();
            assert!(err.contains("rejected by the network-only sandbox"), "unexpected error for {bad}: {err}");
        }
    }

    #[test]
    fn allows_pure_computation() {
        let mut c = ctx();
        let res = run(
            &mut c,
            &json!({ "code": "import json\nrows = [{'v': i} for i in range(5)]\nprint(json.dumps({'n': len(rows), 'sum': sum(r['v'] for r in rows)}))" }),
        )
        .unwrap();
        assert_eq!(res["ok"], json!(true));
        assert!(res["stdout"].as_str().unwrap().contains("\"sum\": 10"));
    }

    #[test]
    fn syntax_error_is_reported_with_line() {
        let mut c = ctx();
        let err = run(&mut c, &json!({ "code": "def f(:\n  pass" })).unwrap_err().to_string();
        assert!(err.contains("syntax error"), "{err}");
    }

    #[test]
    fn runtime_error_is_structured_not_a_crash() {
        let mut c = ctx();
        let res = run(&mut c, &json!({ "code": "print(1/0)" })).unwrap();
        assert_eq!(res["ok"], json!(false));
        assert!(res["error"].as_str().unwrap().contains("ZeroDivisionError"));
    }

    #[test]
    fn infinite_loop_is_killed() {
        let mut c = ctx();
        let res = run(&mut c, &json!({ "code": "while True:\n    pass", "timeout_secs": 3 })).unwrap();
        assert_eq!(res["ok"], json!(false), "{res}");
    }
}
