//! VeriScout (ves) — a web research, source-verification and visualization agent
//! whose tool surface is exactly six purpose-built tools. There is no shell, no file
//! editor and no generic execution primitive anywhere in this binary.

mod agent;
mod blacklist;
mod cli;
mod config;
mod html;
mod http;
mod llm;
mod mcp;
mod proc;
mod search;
mod tools;
mod util;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::main(argv);
    std::process::exit(code);
}
