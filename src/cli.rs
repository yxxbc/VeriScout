//! Command line interface: one-shot runs, interactive REPL, and direct tool access.

use crate::agent::{paint, Agent};
use crate::config::{expand_tilde, redacted_summary, Config, VERSION};
use crate::search::{Aggregator, Scope};
use crate::tools::{self, html_tools, Ctx};
use crate::util;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, IsTerminal, Write};

const USAGE: &str = concat!(
    "ves ",
    env!("CARGO_PKG_VERSION"),
    r#" - VeriScout: web research, source verification and visualization

USAGE
  ves [options] <question...>        run one research task (default command)
  ves repl                           interactive session (multi-turn)
  ves mcp                            MCP stdio server: this project as a tool for other agents
  ves tools [--json]                 list the sealed tool surface
  ves call <tool> [--args JSON]      invoke one tool directly, no model involved
  ves search <query> [options]       search only (no model)
  ves fetch <url> [options]          fetch one URL through the hardened curl tool
  ves script (--code S | --file F)   run a network-only Python script
  ves viz list|path|open [name]      manage generated visualizations
  ves config show|init|path          configuration
  ves help | version

OPTIONS
  -c, --config <path>    config file (default ./.ves/config.json, then ~/.ves/config.json)
  -m, --model <name>     override the model
      --max-steps <n>    override the agent step budget
      --base-url <url>   override the OpenAI-compatible endpoint
      --scope <s>        search scope: web|news|academic|code|qa|all
      --limit <n>        max results (search) or pages
      --engines <a,b>    restrict engines
      --deep             also fetch the top result pages
      --raw              fetch: return raw body instead of extracted text
      --headers          fetch: include response headers
      --save             fetch: save the raw body under raw/
      --render           fetch: render a JS-heavy page via the AnySearch extractor
      --theme <t>        create_html: dark|light
      --html             also write the offline HTML report (default: off)
      --no-html          force the report off even when viz.auto_html is true
      --json             machine-readable output
  -v, --verbose          also print the full closing answer and every tool result
  -q, --quiet            suppress the progress trace: print only the answer/report link
      --no-color         disable ANSI colours
  -V, --version          print version
  -h, --help             print this help

NOTES
  By default the agent answers in the terminal: it searches, opens the primary
  sources and cross-checks, then prints a Markdown answer (headings, lists, links,
  code blocks) rendered for the terminal — nothing is written to disk. Add --html
  (or set "viz.auto_html": true) to also get the offline report in visualizations/;
  the three report tools are only offered to the model in that mode, so a default
  run cannot surprise you with files.

  The agent has no shell, no file access and no system tools: the tool list is the
  surface, and by default that surface is three tools — web_search, curl_tool,
  scripts_tool.

  Colours follow the terminal (NO_COLOR=1 / --no-color disable them). OSC 8
  hyperlinks are used for links and report paths where the terminal supports them
  (FORCE_HYPERLINK=0/1 overrides). `-v` prints the full closing answer and every
  tool result.
"#);

#[derive(Debug, Default)]
pub struct Cli {
    pub config_path: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub max_steps: Option<u32>,
    pub json: bool,
    pub verbose: bool,
    pub quiet: bool,
    pub color: bool,
    pub command: String,
    pub positional: Vec<String>,
    pub flags: BTreeMap<String, String>,
    pub switches: BTreeSet<String>,
}

const VALUE_FLAGS: [&str; 13] = [
    "config", "model", "base-url", "max-steps", "scope", "limit", "engines", "args", "args-file",
    "code", "file", "timeout", "theme",
];

pub fn parse(argv: Vec<String>) -> Result<Cli> {
    let mut cli = Cli {
        color: std::io::stdout().is_terminal() && std::env::var("NO_COLOR").is_err(),
        command: String::new(),
        ..Default::default()
    };
    let mut i = 0usize;
    let mut words: Vec<String> = Vec::new();
    while i < argv.len() {
        let arg = argv[i].clone();
        if let Some(rest) = arg.strip_prefix("--") {
            let (key, inline) = match rest.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (rest.to_string(), None),
            };
            if VALUE_FLAGS.contains(&key.as_str()) {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        argv.get(i).cloned().ok_or_else(|| anyhow!("--{key} requires a value"))?
                    }
                };
                cli.flags.insert(key, value);
            } else if key == "json" {
                cli.json = true;
            } else if key == "verbose" {
                cli.verbose = true;
            } else if key == "quiet" {
                cli.quiet = true;
            } else if key == "no-color" {
                cli.color = false;
            } else {
                cli.switches.insert(key);
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg.trim_start_matches('-').chars() {
                match ch {
                    'c' => {
                        i += 1;
                        cli.flags.insert("config".into(), argv.get(i).cloned().ok_or_else(|| anyhow!("-c requires a value"))?);
                    }
                    'm' => {
                        i += 1;
                        cli.flags.insert("model".into(), argv.get(i).cloned().ok_or_else(|| anyhow!("-m requires a value"))?);
                    }
                    'v' => cli.verbose = true,
                    'q' => cli.quiet = true,
                    'V' => words.push("version".into()),
                    'h' => words.push("help".into()),
                    other => return Err(anyhow!("unknown flag -{other}")),
                }
            }
        } else {
            words.push(arg);
        }
        i += 1;
    }

    cli.config_path = cli.flags.get("config").cloned();
    cli.model = cli.flags.get("model").cloned();
    cli.base_url = cli.flags.get("base-url").cloned();
    cli.max_steps = cli.flags.get("max-steps").and_then(|v| v.parse().ok());

    const COMMANDS: [&str; 13] = [
        "run", "repl", "mcp", "tools", "call", "search", "fetch", "script", "viz", "config", "version", "help", "tool",
    ];
    if let Some(first) = words.first() {
        if COMMANDS.contains(&first.as_str()) {
            cli.command = first.clone();
            cli.positional = words[1..].to_vec();
        } else {
            cli.command = "run".into();
            cli.positional = words.clone();
        }
    } else {
        cli.command = "repl".into();
    }
    if cli.command == "tool" {
        cli.command = "call".into();
    }
    // Long forms `--help` / `--version` behave exactly like the subcommands and
    // short flags; without this they were parsed as anonymous switches and the
    // process fell through to the REPL.
    if cli.switches.contains("help") {
        cli.command = "help".into();
        cli.positional.clear();
    } else if cli.switches.contains("version") {
        cli.command = "version".into();
        cli.positional.clear();
    }
    Ok(cli)
}

fn build_config(cli: &Cli) -> Result<Config> {
    let (mut cfg, _path) = Config::load(cli.config_path.as_deref())?;
    if let Some(m) = &cli.model {
        cfg.llm.model = m.clone();
    }
    if let Some(b) = &cli.base_url {
        cfg.llm.base_url = b.clone();
    }
    if let Some(s) = cli.max_steps {
        cfg.agent.max_steps = s;
    }
    if let Some(n) = cli.flags.get("limit").and_then(|v| v.parse::<usize>().ok()) {
        cfg.search.results_per_engine = n;
    }
    // The report is opt-in: `--html` turns it on for one run, `--no-html` overrides a
    // config that enabled it. Default comes from `viz.auto_html` (false).
    if cli.switches.contains("html") {
        cfg.viz.auto_html = true;
    }
    if cli.switches.contains("no-html") {
        cfg.viz.auto_html = false;
    }
    Ok(cfg)
}

pub fn main(argv: Vec<String>) -> i32 {
    let cli = match parse(argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ves: {e}\n\n{USAGE}");
            return 2;
        }
    };
    let result = match cli.command.as_str() {
        "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "version" => {
            println!("ves {VERSION}");
            Ok(())
        }
        "config" => cmd_config(&cli),
        "tools" => cmd_tools(&cli, None),
        "call" => cmd_call(&cli),
        "search" => cmd_search(&cli),
        "fetch" => cmd_fetch(&cli),
        "script" => cmd_script(&cli),
        "viz" => cmd_viz(&cli),
        "mcp" => cmd_mcp(&cli),
        "repl" => cmd_repl(&cli),
        _ => cmd_run(&cli),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let color = cli.color;
            eprintln!("{}", paint(color, "1;31", &format!("ves: {e}")));
            1
        }
    }
}

fn cmd_run(cli: &Cli) -> Result<()> {
    let query = cli.positional.join(" ");
    if query.trim().is_empty() {
        return Err(anyhow!("no question given. Try: ves \"your question\""));
    }
    let cfg = build_config(cli)?;
    let trace = !cli.quiet;
    let mut agent = Agent::new(cfg, trace, cli.color)?;
    agent.set_verbose(cli.verbose);
    if !cli.quiet {
        eprintln!(
            "{}",
            paint(
                cli.color,
                "2",
                &format!(
                    "ves {} · model {} · 报告 {}",
                    VERSION,
                    agent.model_line(),
                    if agent.auto_html() { "on（写入 visualizations/）" } else { "off（--html 打开）" }
                )
            )
        );
    }
    let outcome = agent.run(&query)?;
    let finalized = close_reports(&mut agent, &outcome);
    print_outcome(&outcome, cli, &agent, &finalized)
}

/// Hand the run's detail to the report before anything is printed: the closing
/// answer, the tool log and the timing all belong to the artifact — which is why
/// the terminal only has to point at it.
fn close_reports(agent: &mut Agent, outcome: &crate::agent::RunOutcome) -> Vec<html_tools::FinalizedReport> {
    html_tools::finalize(
        &mut agent.ctx,
        &outcome.new_reports,
        &outcome.answer,
        outcome.steps,
        outcome.elapsed_ms,
    )
}

fn print_outcome(
    outcome: &crate::agent::RunOutcome,
    cli: &Cli,
    agent: &Agent,
    finalized: &[html_tools::FinalizedReport],
) -> Result<()> {
    if cli.json {
        let payload = json!({
            "answer": outcome.answer,
            "steps": outcome.steps,
            "tool_calls": outcome.tool_calls,
            "reports": outcome.new_reports.iter().map(|n| report_paths(agent, n)).collect::<Vec<_>>(),
            "report_stats": finalized
                .iter()
                .map(|f| json!({
                    "name": f.name,
                    "items": f.items,
                    "sources": f.sources,
                    "verified": f.verified,
                    "flagged": f.flagged,
                    "avg_reliability": (f.avg_reliability * 10.0).round() / 10.0,
                }))
                .collect::<Vec<_>>(),
            "usage": {
                "prompt_tokens": outcome.usage.prompt_tokens,
                "completion_tokens": outcome.usage.completion_tokens,
                "total_tokens": outcome.usage.total_tokens,
            },
            "stopped_reason": outcome.stopped_reason,
            "elapsed_ms": outcome.elapsed_ms,
            "model": agent.model_line(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    // The answer is the default deliverable now: it is printed here, rendered as
    // Markdown. A run that also produced a report still prints nothing extra — the
    // answer went into report §1 — unless `-v` asked for the full text.
    let has_report = !finalized.is_empty();
    if !has_report && !cli.quiet && agent.auto_html() {
        eprintln!(
            "{}",
            paint(cli.color, "33", "⚠ 开了报告但这次没生成：结论直接打印，完整轨迹加 -v")
        );
    }
    if let Some(answer) = answer_for_terminal(&outcome.answer, cli.verbose, has_report) {
        println!("{}", crate::md::render(answer, cli.color));
    }

    if has_report {
        println!();
        for f in finalized {
            println!("{}", report_block(agent, f, cli));
        }
        // A report without a single claim is a silent failure mode: the model used to
        // mention it in prose, and prose no longer reaches the terminal — say it here.
        if !cli.quiet {
            for f in empty_reports(finalized) {
                eprintln!(
                    "{}",
                    paint(
                        cli.color,
                        "33",
                        &format!("⚠ {}.html 里没有写进任何主张（claims）：可在会话里让它用 edit_html 补，或重新生成", f.name)
                    )
                );
            }
        }
    }

    if !cli.quiet {
        eprintln!(
            "{}",
            paint(
                cli.color,
                "2",
                &format!(
                    "— {} 步 · {} 次工具调用 · {} tokens · {} ms · {}",
                    outcome.steps,
                    outcome.tool_calls,
                    outcome.usage.total_tokens,
                    outcome.elapsed_ms,
                    outcome.stopped_reason
                )
            )
        );
    }
    Ok(())
}

/// The terminal is an index card, not the dossier. A report run hands the closing
/// answer to the artifact (§1 结论) and stays silent here; `-v` prints the full text.
/// Everything else — the default report-free run, and any run whose report failed —
/// prints it, because it is the only deliverable there is.
fn answer_for_terminal(answer: &str, verbose: bool, has_report: bool) -> Option<&str> {
    let answer = answer.trim();
    if answer.is_empty() {
        return None;
    }
    if verbose || !has_report {
        Some(answer)
    } else {
        None
    }
}

/// Reports that finished with zero claims. The terminal has to flag them itself:
/// the model's own "the claims section is empty" complaint is prose, and prose no
/// longer reaches the terminal.
fn empty_reports(finalized: &[html_tools::FinalizedReport]) -> Vec<&html_tools::FinalizedReport> {
    finalized.iter().filter(|f| f.items == 0).collect()
}

/// One report, three lines: the clickable name (OSC 8), a machine-checkable
/// audit summary, and the plain path so the file can be copied anywhere.
fn report_block(agent: &Agent, f: &html_tools::FinalizedReport, cli: &Cli) -> String {
    let path = agent.ctx.cfg.viz_dir().join(format!("{}.html", f.name));
    let url = util::file_url(&path);
    let links = util::hyperlinks_enabled(cli.color);
    let name = format!("{}.html", f.name);

    let mut counts = vec![format!("{} 条说法", f.items), format!("{} 个来源", f.sources)];
    if f.verified > 0 {
        counts.push(format!("{} 条有来源支持", f.verified));
    }
    if f.flagged > 0 {
        counts.push(format!("{} 条有矛盾/被否认", f.flagged));
    }
    if f.avg_reliability > 0.0 {
        counts.push(format!("来源平均可信度 {:.0}/100", f.avg_reliability));
    }

    let mut lines = vec![
        format!("📄 报告  {}", util::hyperlink(&url, &paint(cli.color, "1;36", &name), links)),
        format!("   {}", paint(cli.color, "2", &counts.join(" · "))),
        format!("   {}", util::hyperlink(&url, &paint(cli.color, "2", &path.display().to_string()), links)),
    ];
    if !cli.verbose {
        lines.push(format!(
            "   {}",
            paint(cli.color, "2", "↳ 模型原话 · 逐条核对 · 查证过程都在报告里，打开就能看")
        ));
    }
    lines.join("\n")
}

fn report_paths(agent: &Agent, name: &str) -> (String, String) {
    let dir = agent.ctx.cfg.viz_dir();
    (
        dir.join(format!("{name}.html")).display().to_string(),
        dir.join(format!("{name}.json")).display().to_string(),
    )
}

fn direct_ctx(cli: &Cli) -> Result<Ctx> {
    Ok(Ctx::new(build_config(cli)?))
}

fn print_json(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(_) => println!("{value}"),
    }
}

fn cmd_tools(cli: &Cli, auto_html: Option<bool>) -> Result<()> {
    let mut cfg = build_config(cli)?;
    if let Some(on) = auto_html {
        cfg.viz.auto_html = on;
    }
    let surface = tools::surface(&cfg);
    if cli.json {
        print_json(&tools::openai_tools_for_config(&cfg));
        return Ok(());
    }
    let mode = if cfg.viz.auto_html {
        "含报告工具"
    } else {
        "默认不含报告工具 · 加 --html 或设 viz.auto_html=true 打开"
    };
    println!(
        "{}",
        paint(
            cli.color,
            "1",
            &format!("VeriScout 工具面：{} 个 · {} · 无 shell / 无文件读写 / 无通用执行", surface.len(), mode)
        )
    );
    println!();
    for (name, desc, required) in tools::describe_for(&surface) {
        println!("{}", paint(cli.color, "1;36", &format!("● {name}")));
        println!("  {}", first_sentence(desc));
        if !required.is_empty() {
            println!("  {}", paint(cli.color, "2", &format!("required: {}", required.join(", "))));
        }
        println!();
    }
    Ok(())
}

fn first_sentence(desc: &str) -> String {
    let cut = desc.split(". ").next().unwrap_or(desc);
    let cut = cut.split("。").next().unwrap_or(cut);
    cut.trim().to_string()
}

fn cmd_call(cli: &Cli) -> Result<()> {
    let name = cli
        .positional
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("usage: ves call <tool> [--args JSON] [--args-file FILE]"))?;
    let raw = if let Some(a) = cli.flags.get("args") {
        a.clone()
    } else if let Some(path) = cli.flags.get("args-file") {
        std::fs::read_to_string(expand_tilde(path))?
    } else if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        buf
    } else {
        "{}".to_string()
    };
    let args: Value = if raw.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&raw).map_err(|e| anyhow!("--args is not valid JSON: {e}"))?
    };
    let mut ctx = direct_ctx(cli)?;
    let result = tools::dispatch(&mut ctx, &name, &args)?;
    print_json(&result);
    Ok(())
}

fn cmd_search(cli: &Cli) -> Result<()> {
    let query = cli.positional.join(" ");
    if query.trim().is_empty() {
        return Err(anyhow!("usage: ves search <query>"));
    }
    let ctx = direct_ctx(cli)?;
    let scope = Scope::parse(cli.flags.get("scope").map(|s| s.as_str()).unwrap_or("web"));
    let limit = cli.flags.get("limit").and_then(|v| v.parse().ok()).unwrap_or(12usize);
    let engines: Option<Vec<String>> = cli
        .flags
        .get("engines")
        .map(|e| e.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
    if cli.json {
        // identical shape to the web_search tool, so scripts can treat both the same
        let mut mutable_ctx = ctx;
        let args = json!({
            "query": query,
            "scope": scope.label(),
            "limit": limit,
            "engines": engines.clone().unwrap_or_default(),
            "deep": cli.switches.contains("deep"),
        });
        let result = tools::dispatch(&mut mutable_ctx, "web_search", &args)?;
        print_json(&result);
        return Ok(());
    }
    let aggregator = Aggregator::new(&ctx.cfg, &ctx.http);
    let report = aggregator.search(&query, scope, limit, engines.as_deref());
    println!("{}", paint(cli.color, "1", &format!("检索：{query}  ({}, {} ms)", report.scope, report.took_ms)));
    for status in &report.engines {
        let tone = if status.ok { "32" } else { "31" };
        let detail = status.error.clone().unwrap_or_default();
        println!(
            "  {}",
            paint(cli.color, tone, &format!("{:<12} {} results {}ms {}", status.engine, status.results, status.ms, detail))
        );
    }
    if !report.filtered.is_empty() {
        println!("{}", paint(cli.color, "33", &format!("  黑名单已过滤 {} 条：{}", report.filtered.len(), report.filtered.join(" / "))));
    }
    println!();
    for (i, r) in report.results.iter().enumerate() {
        println!("{:>2}. {}", i + 1, paint(cli.color, "1", &r.title));
        println!("    {}", paint(cli.color, "36", &r.url));
        let mut meta = format!("{} · trust {} · score {} · 引擎 {}", r.domain, r.trust, r.score, r.engine);
        if r.corroboration > 1 {
            meta.push_str(&format!(" · {} 个来源一致", r.corroboration));
        }
        if let Some(p) = &r.published {
            meta.push_str(&format!(" · {p}"));
        }
        println!("    {}", paint(cli.color, "2", &meta));
        if !r.snippet.is_empty() {
            let (snip, _) = util::truncate_chars(&r.snippet.replace('\n', " "), 180);
            println!("    {}", snip);
        }
        println!();
    }
    Ok(())
}

fn cmd_fetch(cli: &Cli) -> Result<()> {
    let url = cli.positional.first().cloned().ok_or_else(|| anyhow!("usage: ves fetch <url>"))?;
    let mut ctx = direct_ctx(cli)?;
    let args = json!({
        "url": url,
        "extract_text": !cli.switches.contains("raw"),
        "include_headers": cli.switches.contains("headers"),
        "save": cli.switches.contains("save"),
        "render": cli.switches.contains("render"),
        "max_bytes": cli.flags.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(ctx.cfg.curl.max_bytes),
    });
    let result = tools::dispatch(&mut ctx, "curl_tool", &args)?;
    if cli.json {
        print_json(&result);
        return Ok(());
    }
    let status = result.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
    let ct = result.get("content_type").and_then(|v| v.as_str()).unwrap_or("");
    println!("{}", paint(cli.color, "1", &format!("HTTP {status} · {ct}")));
    if let Some(headers) = result.get("headers").and_then(|v| v.as_array()) {
        for h in headers {
            println!("  {}: {}", h["name"].as_str().unwrap_or(""), h["value"].as_str().unwrap_or(""));
        }
    }
    if let Some(text) = result.get("text").and_then(|v| v.as_str()) {
        println!();
        println!("{text}");
    }
    Ok(())
}

fn cmd_script(cli: &Cli) -> Result<()> {
    let code = if let Some(c) = cli.flags.get("code") {
        c.clone()
    } else if let Some(path) = cli.flags.get("file") {
        std::fs::read_to_string(expand_tilde(path))?
    } else if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        buf
    } else {
        return Err(anyhow!("usage: ves script --code '...' | --file script.py | < stdin"));
    };
    let mut ctx = direct_ctx(cli)?;
    let args = json!({
        "code": code,
        "timeout_secs": cli.flags.get("timeout").and_then(|v| v.parse::<u64>().ok()).unwrap_or(ctx.cfg.scripts.timeout_secs),
    });
    let result = tools::dispatch(&mut ctx, "scripts_tool", &args)?;
    if cli.json {
        print_json(&result);
        return Ok(());
    }
    if let Some(out) = result.get("stdout").and_then(|v| v.as_str()) {
        print!("{out}");
        if !out.ends_with('\n') && !out.is_empty() {
            println!();
        }
    }
    if let Some(err) = result.get("stderr").and_then(|v| v.as_str()) {
        if !err.trim().is_empty() {
            eprintln!("{}", paint(cli.color, "31", err.trim_end()));
        }
    }
    let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        return Err(anyhow!(
            "script failed: {}",
            result.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error")
        ));
    }
    Ok(())
}

fn cmd_viz(cli: &Cli) -> Result<()> {
    let action = cli.positional.first().map(|s| s.as_str()).unwrap_or("list");
    let ctx = direct_ctx(cli)?;
    match action {
        "list" => {
            let reports = html_tools::list_reports(&ctx);
            if cli.json {
                print_json(&json!(reports));
            } else if reports.is_empty() {
                println!("（还没有生成任何可视化报告）");
            } else {
                for name in reports {
                    let path = ctx.cfg.viz_dir().join(format!("{name}.html"));
                    let links = util::hyperlinks_enabled(cli.color);
                    let url = util::file_url(&path);
                    println!(
                        "{}  {}",
                        util::hyperlink(&url, &paint(cli.color, "36", &name), links),
                        paint(cli.color, "2", &path.display().to_string())
                    );
                }
            }
            Ok(())
        }
        "path" => {
            println!("{}", ctx.cfg.viz_dir().display());
            Ok(())
        }
        "open" => {
            let name = cli.positional.get(1).cloned().ok_or_else(|| anyhow!("usage: ves viz open <name>"))?;
            let path = ctx.cfg.viz_dir().join(format!("{name}.html"));
            if !path.exists() {
                return Err(anyhow!("no report named '{name}' in {}", ctx.cfg.viz_dir().display()));
            }
            println!("{}", path.display());
            #[cfg(target_os = "macos")]
            let opener = "open";
            #[cfg(target_os = "linux")]
            let opener = "xdg-open";
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            let opener = "";
            if !opener.is_empty() {
                let _ = std::process::Command::new(opener).arg(&path).spawn();
            }
            Ok(())
        }
        other => Err(anyhow!("unknown viz action '{other}' (list | path | open <name>)")),
    }
}

fn cmd_config(cli: &Cli) -> Result<()> {
    let action = cli.positional.first().map(|s| s.as_str()).unwrap_or("show");
    match action {
        "show" => {
            let cfg = build_config(cli)?;
            let summary = redacted_summary(&cfg);
            let (found, default) = Config::resolve_path(cli.config_path.as_deref());
            if cli.json {
                print_json(&json!({
                    "config_file": found.map(|p| p.display().to_string()),
                    "default_path": default.display().to_string(),
                    "settings": summary,
                }));
                return Ok(());
            }
            match found {
                Some(p) => println!("配置文件：{}", p.display()),
                None => println!("配置文件：未找到（正在使用内置默认值，可 ves config init 生成）"),
            }
            println!();
            for (k, v) in summary {
                println!("  {:<22} {}", k, v);
            }
            Ok(())
        }
        "path" => {
            let (found, default) = Config::resolve_path(cli.config_path.as_deref());
            println!("{}", found.unwrap_or(default).display());
            Ok(())
        }
        "init" => {
            let (_, default) = Config::resolve_path(cli.config_path.as_deref());
            if default.exists() && !cli.switches.contains("force") {
                return Err(anyhow!("{} already exists (use --force to overwrite)", default.display()));
            }
            let cfg = Config::default();
            cfg.save(&default)?;
            println!("已写入 {}", default.display());
            Ok(())
        }
        other => Err(anyhow!("unknown config action '{other}' (show | init | path)")),
    }
}

/// `ves mcp`: serve the sealed tool surface (plus a full-research entry point)
/// over the Model Context Protocol on stdin/stdout so other agents can call this
/// project as a tool. stdout carries protocol traffic only; logs go to stderr.
fn cmd_mcp(cli: &Cli) -> Result<()> {
    let cfg = build_config(cli)?;
    crate::mcp::serve(cfg)
}

fn cmd_repl(cli: &Cli) -> Result<()> {
    let cfg = build_config(cli)?;
    let mut agent = Agent::new(cfg, true, cli.color)?;
    agent.set_verbose(cli.verbose);
    if !cli.quiet {
        println!("{}", paint(cli.color, "1;36", &format!("VeriScout (ves {}) — 检索 · 溯源 · 出结论", VERSION)));
        println!(
            "{}",
            paint(
                cli.color,
                "2",
                &format!(
                    "model: {} · 报告 {}",
                    agent.model_line(),
                    if agent.auto_html() { "on（写入 visualizations/）" } else { "off（/html on 打开）" }
                )
            )
        );
        println!("{}", paint(cli.color, "2", "输入问题开始研究；/help 查看命令，/exit 退出。"));
        println!();
    }
    let stdin = std::io::stdin();
    loop {
        print!("{}", paint(cli.color, "1;32", "ves> "));
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            println!();
            break;
        }
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('/') {
            let mut parts = rest.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("").trim();
            let arg = parts.next().unwrap_or("").trim().to_string();
            match cmd {
                "exit" | "quit" | "q" => break,
                "help" | "?" => print_repl_help(cli.color),
                "new" => {
                    agent.reset();
                    println!("{}", paint(cli.color, "2", "已重置会话上下文。"));
                }
                "tools" => cmd_tools(cli, Some(agent.auto_html()))?,
                "config" => cmd_config(cli)?,
                "viz" => {
                    for name in html_tools::list_reports(&agent.ctx) {
                        println!("  {}", name);
                    }
                }
                "html" => match arg.as_str() {
                    "on" | "1" | "true" | "yes" => {
                        agent.set_auto_html(true);
                        println!("  html = on（每轮都会生成报告到 visualizations/）");
                    }
                    "off" | "0" | "false" | "no" => {
                        agent.set_auto_html(false);
                        println!("  html = off（只在终端回答）");
                    }
                    _ => println!(
                        "  html = {}（用法：/html on|off）",
                        if agent.auto_html() { "on" } else { "off" }
                    ),
                },
                "steps" => match arg.parse::<u32>() {
                    Ok(n) => {
                        agent.set_max_steps(n);
                        println!("  max_steps = {n}");
                    }
                    Err(_) => println!("  用法：/steps 12"),
                },
                "trace" => match arg.as_str() {
                    "full" | "all" => {
                        agent.set_trace(true);
                        agent.set_verbose(true);
                        println!("  trace = full（含每步 token 与工具返回预览）");
                    }
                    "off" | "0" | "false" => {
                        agent.set_trace(false);
                        println!("  trace = off");
                    }
                    _ => {
                        agent.set_trace(true);
                        agent.set_verbose(false);
                        println!("  trace = on（精简；/trace full 打印完整轨迹）");
                    }
                },
                other => println!("  未知命令 /{other}（/help 查看）"),
            }
            continue;
        }
        match agent.run(&trimmed) {
            Ok(outcome) => {
                let finalized = close_reports(&mut agent, &outcome);
                print_outcome(&outcome, cli, &agent, &finalized)?;
            }
            Err(e) => eprintln!("{}", paint(cli.color, "1;31", &format!("运行失败：{e}"))),
        }
    }
    Ok(())
}

fn print_repl_help(color: bool) {
    println!("{}", paint(color, "1", "命令："));
    println!("  /new            清空对话上下文");
    println!("  /tools          查看可用工具");
    println!("  /html on|off    是否生成 HTML 报告（默认 off：只在终端回答）");
    println!("  /viz            列出已生成的报告");
    println!("  /steps <n>      设置最大步数");
    println!("  /trace on|full|off  精简轨迹 / 完整轨迹 / 关闭轨迹");
    println!("  /config         显示配置");
    println!("  /exit           退出");
    println!("直接输入问题即开始一轮研究。");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_to_repl_then_run() {
        assert_eq!(parse(args(&[])).unwrap().command, "repl");
        let cli = parse(args(&["北京", "今天", "天气"])).unwrap();
        assert_eq!(cli.command, "run");
        assert_eq!(cli.positional.join(" "), "北京 今天 天气");
    }

    #[test]
    fn parses_flags_and_switches() {
        let cli = parse(args(&["search", "rust", "--scope", "news", "--limit=5", "--json", "-v"])).unwrap();
        assert_eq!(cli.command, "search");
        assert_eq!(cli.flags.get("scope").unwrap(), "news");
        assert_eq!(cli.flags.get("limit").unwrap(), "5");
        assert!(cli.json && cli.verbose);
    }

    #[test]
    fn long_help_and_version_work_like_the_short_forms() {
        assert_eq!(parse(args(&["--version"])).unwrap().command, "version");
        assert_eq!(parse(args(&["--help"])).unwrap().command, "help");
        assert_eq!(parse(args(&["-V"])).unwrap().command, "version");
        assert_eq!(parse(args(&["-h"])).unwrap().command, "help");
    }

    #[test]
    fn html_flags_toggle_the_report_surface() {
        // The default lives in the config, so assert it on the struct itself and test
        // the *flag override* here — that keeps this test independent of any
        // ./.ves/config.json or ~/.ves/config.json on the machine running it.
        assert!(!Config::default().viz.auto_html, "the HTML report must be opt-in by default");
        assert_eq!(tools::surface(&Config::default()).len(), 3, "default surface is research-only");

        let on = build_config(&parse(args(&["--html", "问题"])).unwrap()).unwrap();
        assert!(on.viz.auto_html);
        assert_eq!(tools::surface(&on).len(), 6);

        let off = build_config(&parse(args(&["--html", "--no-html", "问题"])).unwrap()).unwrap();
        assert!(!off.viz.auto_html, "--no-html must win over --html");

        // a flag must not steal the question
        let run = parse(args(&["--html", "北京", "天气"])).unwrap();
        assert_eq!(run.command, "run");
        assert_eq!(run.positional.join(" "), "北京 天气");
        assert!(run.switches.contains("html"));
    }

    #[test]
    fn the_answer_is_rendered_as_markdown() {
        let rendered = crate::md::render("## 结论\n\n- 要点一\n- 要点二", false);
        assert!(rendered.contains("结论"), "{rendered:?}");
        assert!(!rendered.contains("##"), "ATX markers must not reach the terminal: {rendered:?}");
        assert!(rendered.contains("要点一"));
    }

    #[test]
    fn report_block_is_a_clickable_pointer_to_the_artifact() {
        let mut cfg = Config::default();
        // Keep the test hermetic: an inline key means it does not depend on an env
        // var or a credential file being present on the machine running it.
        cfg.llm.api_key = "sk-test".into();
        cfg.llm.credential_files.clear();
        let agent = Agent::new(cfg, false, false).unwrap();
        let finalized = html_tools::FinalizedReport {
            name: "demo-report".into(),
            items: 12,
            sources: 19,
            verified: 9,
            flagged: 2,
            avg_reliability: 78.4,
        };
        let mut cli = Cli { color: false, ..Default::default() };
        let plain = report_block(&agent, &finalized, &cli);
        assert!(plain.contains("demo-report.html"), "{plain}");
        assert!(plain.contains("12 条说法"), "counts must be visible: {plain}");
        assert!(plain.contains("9 条有来源支持"), "{plain}");
        assert!(plain.contains("2 条有矛盾/被否认"), "{plain}");
        assert!(plain.contains("来源平均可信度 78"), "{plain}");
        assert!(plain.contains("原话"), "detail pointer must be shown");
        assert!(!plain.contains("\u{1b}]8;;"), "no escape sequences when colour is off");

        // With links enabled the path is wrapped in an OSC 8 hyperlink, so the
        // terminal opens it on Cmd/Ctrl+click.
        cli.color = true;
        if util::hyperlinks_enabled(true) {
            let linked = report_block(&agent, &finalized, &cli);
            assert!(linked.contains("\u{1b}]8;;file:///"), "the path must be a file:// hyperlink: {linked:?}");
            assert!(linked.contains("\u{1b}]8;;\u{1b}\\"), "the hyperlink must be terminated");
        }
    }

    #[test]
    fn terminal_defers_the_answer_to_the_report() {
        // The closing answer is written into report §1, so a run that produced one
        // prints nothing of it in the terminal; -v overrides, and a report-less run
        // prints the full text because there is nowhere else to read it.
        let answer = "# 本周结论\n\n要点一：……";
        assert_eq!(answer_for_terminal(answer, false, true), None);
        assert_eq!(answer_for_terminal(answer, true, true), Some(answer));
        assert_eq!(answer_for_terminal(answer, false, false), Some(answer));
        assert_eq!(answer_for_terminal("  \n ", false, false), None);
        assert_eq!(answer_for_terminal("  纯文本  ", false, false), Some("纯文本"));
    }

    #[test]
    fn empty_claim_sections_are_flagged_by_the_terminal() {
        let report = |name: &str, items: usize| html_tools::FinalizedReport {
            name: name.into(),
            items,
            sources: 2,
            verified: 0,
            flagged: 0,
            avg_reliability: 50.0,
        };
        let all = vec![report("full", 3), report("hollow", 0)];
        let flagged: Vec<&str> = empty_reports(&all).iter().map(|f| f.name.as_str()).collect();
        assert_eq!(flagged, vec!["hollow"], "only claim-less reports may be flagged");
    }

    #[test]
    fn short_flags_take_values() {
        let cli = parse(args(&["-c", "/tmp/x.json", "-m", "gpt", "hi"])).unwrap();
        assert_eq!(cli.config_path.as_deref(), Some("/tmp/x.json"));
        assert_eq!(cli.model.as_deref(), Some("gpt"));
        assert_eq!(cli.command, "run");
    }

    #[test]
    fn missing_flag_value_is_an_error() {
        assert!(parse(args(&["--config"])).is_err());
    }

    #[test]
    fn tool_alias_is_accepted() {
        assert_eq!(parse(args(&["tool", "web_search"])).unwrap().command, "call");
    }

    #[test]
    fn mcp_command_is_recognised() {
        assert_eq!(parse(args(&["mcp"])).unwrap().command, "mcp");
    }
}
