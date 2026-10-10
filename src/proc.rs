//! Small helper for running a local executable with a hard wall-clock limit.
//!
//! Used by anything that must shell out to a trusted local CLI (the AnySearch helper
//! script, the Python sandbox). Arguments are always passed as an argv array - a shell
//! is never involved, so no input can be interpreted as a command.

use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub timeout: Duration,
    pub stdin: Option<String>,
    /// Maximum bytes captured per stream.
    pub output_limit: usize,
}

impl RunOptions {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout, stdin: None, output_limit: 256 * 1024 }
    }

    #[allow(dead_code)]
    pub fn stdin(mut self, text: Option<String>) -> Self {
        self.stdin = text;
        self
    }

    pub fn output_limit(mut self, bytes: usize) -> Self {
        self.output_limit = bytes.max(1024);
        self
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration_ms: u128,
}

impl Output {
    pub fn ok(&self) -> bool {
        !self.timed_out && self.exit_code == 0
    }

    /// Last meaningful line of stderr, for error messages.
    pub fn stderr_tail(&self) -> String {
        self.stderr
            .lines()
            .filter(|l| !l.trim().is_empty())
            .next_back()
            .unwrap_or("")
            .trim()
            .to_string()
    }
}

/// Spawn `cmd`, drain its output, and kill it if it exceeds the deadline.
pub fn run(cmd: &mut Command, opts: &RunOptions) -> Result<Output> {
    cmd.stdin(if opts.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| anyhow!("cannot start process: {e}"))?;

    let limit = opts.output_limit;
    let out_pipe = child.stdout.take().ok_or_else(|| anyhow!("no stdout pipe"))?;
    let err_pipe = child.stderr.take().ok_or_else(|| anyhow!("no stderr pipe"))?;
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

    if let Some(text) = &opts.stdin {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(text.as_bytes());
        }
    }

    let deadline = started + opts.timeout;
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break -1;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(anyhow!("waiting for process failed: {e}")),
        }
    };

    let stdout = String::from_utf8_lossy(&out_handle.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&err_handle.join().unwrap_or_default()).to_string();
    Ok(Output {
        exit_code,
        stdout,
        stderr,
        timed_out,
        duration_ms: started.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_exit_code() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("echo hello; echo warn >&2; exit 0");
        let out = run(&mut cmd, &RunOptions::new(Duration::from_secs(10))).unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout.trim(), "hello");
        assert_eq!(out.stderr.trim(), "warn");
    }

    #[test]
    fn nonzero_exit_is_reported_not_raised() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("exit 3");
        let out = run(&mut cmd, &RunOptions::new(Duration::from_secs(10))).unwrap();
        assert_eq!(out.exit_code, 3);
        assert!(!out.ok());
    }

    #[test]
    fn runaway_process_is_killed() {
        let mut cmd = Command::new("/bin/sh");
        // `exec` keeps the child *as* the sleep process: if the shell forked it
        // instead, the grandchild would hold the pipes open and the reader threads
        // would wait for it (30s) — CI caught exactly that.
        cmd.arg("-c").arg("exec sleep 30");
        let out = run(&mut cmd, &RunOptions::new(Duration::from_millis(400))).unwrap();
        assert!(out.timed_out, "process should have been killed");
        assert!(out.duration_ms < 5_000);
    }

    #[test]
    fn missing_binary_is_a_clean_error() {
        let mut cmd = Command::new("/nonexistent/binary");
        assert!(run(&mut cmd, &RunOptions::new(Duration::from_secs(2))).is_err());
    }

    #[test]
    fn stdin_is_delivered() {
        let mut cmd = Command::new("/bin/cat");
        let out = run(&mut cmd, &RunOptions::new(Duration::from_secs(10)).stdin(Some("piped".into()))).unwrap();
        assert_eq!(out.stdout.trim(), "piped");
    }
}
