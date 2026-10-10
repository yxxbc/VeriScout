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
mod md;
mod proc;
mod search;
mod tools;
mod util;

#[cfg(test)]
mod installer_tests {
    /// Some shells treat the character right after `$VAR` as part of the name when it
    /// is multi-byte: `"（$REPO）"` was read as the variable `REPO）`, and with `set -u`
    /// the installer died with "REPO）: unbound variable" — on the one path a user
    /// takes first (`curl … | sh`, no pinned version). Braces are mandatory there.
    #[test]
    fn installers_brace_variables_before_non_ascii_text() {
        for (name, src) in [
            ("install.sh", include_str!("../install.sh")),
            ("install.ps1", include_str!("../install.ps1")),
        ] {
            if let Some((line_no, offending)) = unbraced_variable_before_non_ascii(src) {
                panic!("{name}:{line_no}: {offending}");
            }
        }
    }

    /// `(line number, description)` of the first `$VAR` that is followed directly by a
    /// non-ASCII character without braces.
    fn unbraced_variable_before_non_ascii(src: &str) -> Option<(usize, String)> {
        for (idx, line) in src.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
                    // braced: skip the whole ${...}
                    i += 2;
                    while i < chars.len() && chars[i] != '}' {
                        i += 1;
                    }
                    i += 1;
                    continue;
                }
                if chars[i] == '$' && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == '_') {
                    let mut j = i + 1;
                    while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    if j < chars.len() && !chars[j].is_ascii() {
                        let var: String = chars[i..j].iter().collect();
                        return Some((idx + 1, format!("{var} is followed by '{}' — use {}{}…", chars[j], var, "{…}")));
                    }
                    i = j;
                    continue;
                }
                i += 1;
            }
        }
        None
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::main(argv);
    std::process::exit(code);
}
