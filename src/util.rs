//! Small dependency-light helpers: time, text, URL and hashing utilities.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days-from-civil algorithm (Howard Hinnant), no external date crate needed.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format an epoch second as an ISO-8601 UTC timestamp (`2026-02-03T04:05:06Z`).
pub fn iso8601(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub fn iso8601_now() -> String {
    iso8601(now_epoch())
}

/// Local wall-clock time with its UTC offset, e.g. `2026-02-03 20:05:06 (UTC+08:00)`.
///
/// Uses libc's `localtime_r` on unix and falls back to UTC elsewhere; kept here
/// (rather than pulling a date crate) so the dependency set stays tiny.
#[cfg(unix)]
pub fn iso8601_local(epoch: i64) -> String {
    unsafe {
        let t = epoch as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return format!("{} (UTC)", iso8601(epoch));
        }
        let offset_min = tm.tm_gmtoff / 60;
        let sign = if offset_min < 0 { '-' } else { '+' };
        let abs = offset_min.abs();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} (UTC{}{:02}:{:02})",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            sign,
            abs / 60,
            abs % 60
        )
    }
}

#[cfg(not(unix))]
pub fn iso8601_local(epoch: i64) -> String {
    format!("{} (UTC)", iso8601(epoch))
}

#[cfg(unix)]
pub fn iso8601_now_local() -> String {
    iso8601_local(now_epoch())
}

#[cfg(not(unix))]
pub fn iso8601_now_local() -> String {
    iso8601_local(now_epoch())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Parse the handful of date shapes that show up in RSS/Atom/JSON feeds.
pub fn parse_flexible_date(s: &str) -> Option<i64> {
    let s = s.trim();
    // RFC-822: "Tue, 03 Feb 2026 04:05:06 GMT"
    let core = match s.find(", ") {
        Some(i) => &s[i + 2..],
        None => s,
    };
    let parts: Vec<&str> = core.split_whitespace().collect();
    if parts.len() >= 4 {
        let day: i64 = parts[0].parse().ok()?;
        let month = MONTHS
            .iter()
            .position(|m| m.eq_ignore_ascii_case(parts[1]))
            .map(|i| i as i64 + 1);
        if let Some(month) = month {
            let year: i64 = parts[2].parse().ok()?;
            let (mut hh, mut mm, mut ss) = (0i64, 0i64, 0i64);
            if let Some(t) = parts.get(3) {
                let tp: Vec<&str> = t.split(':').collect();
                if tp.len() == 3 {
                    hh = tp[0].parse().unwrap_or(0);
                    mm = tp[1].parse().unwrap_or(0);
                    ss = tp[2].parse().unwrap_or(0);
                }
            }
            return Some(days_from_civil(year, month as u32, day as u32) * 86_400 + hh * 3600 + mm * 60 + ss);
        }
    }
    // ISO-8601: "2026-02-03T04:05:06Z" / "2026-02-03"
    let (date, time) = match s.find(['T', ' ']) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    let dp: Vec<&str> = date.split('-').collect();
    if dp.len() == 3 {
        let y: i64 = dp[0].parse().ok()?;
        let m: u32 = dp[1].parse().ok()?;
        let d: u32 = dp[2].parse().ok()?;
        let mut secs = 0i64;
        let tp: Vec<&str> = time.trim_end_matches('Z').split(':').collect();
        if tp.len() >= 2 {
            secs = tp[0].parse().unwrap_or(0) * 3600 + tp[1].parse().unwrap_or(0) * 60;
            if tp.len() >= 3 {
                secs += tp[2].split('.').next().unwrap_or("0").parse().unwrap_or(0);
            }
        }
        return Some(days_from_civil(y, m, d) * 86_400 + secs);
    }
    None
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Filesystem/report-safe slug. Keeps ASCII alphanumerics and CJK, folds the rest to '-'.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in s.chars() {
        let keep = ch.is_ascii_alphanumeric() || is_cjk(ch);
        if keep {
            out.extend(ch.to_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.chars().count() >= 60 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        format!("report-{}", &short_hash(s)[..6])
    } else {
        trimmed
    }
}

pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3000..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

pub fn contains_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}


pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '&' {
            if let Some(semi) = bytes[i..].iter().take(12).position(|c| *c == ';') {
                let ent: String = bytes[i + 1..i + semi].iter().collect();
                let replaced = match ent.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    "mdash" => Some('—'),
                    "ndash" => Some('–'),
                    "hellip" => Some('…'),
                    "middot" => Some('·'),
                    "laquo" => Some('«'),
                    "raquo" => Some('»'),
                    "ldquo" => Some('“'),
                    "rdquo" => Some('”'),
                    "lsquo" => Some('‘'),
                    "rsquo" => Some('’'),
                    _ => {
                        if let Some(num) = ent.strip_prefix('#') {
                            let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                                u32::from_str_radix(hex, 16).ok()
                            } else {
                                num.parse::<u32>().ok()
                            };
                            code.and_then(char::from_u32)
                        } else {
                            None
                        }
                    }
                };
                if let Some(c) = replaced {
                    out.push(c);
                    i += semi + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Crude but fast HTML -> readable text conversion.
///
/// Skips `script`/`style`/`noscript`/`svg`/`head` bodies and turns block-level tags into
/// newlines. Tag matching is boundary-aware: `<header>` must never be mistaken for `<head>`
/// (that mistake would discard the whole document body).
pub fn html_to_text(html: &str) -> String {
    // ASCII-only lowercasing keeps byte offsets identical to the original string.
    let lower = html.to_ascii_lowercase();
    const SKIP: [&str; 5] = ["script", "style", "noscript", "svg", "head"];
    let mut out = String::with_capacity(html.len() / 3);
    let mut cursor = 0usize;
    while cursor < html.len() {
        match find_open_tag(&lower, cursor, &SKIP) {
            Some((pos, tag)) => {
                out.push_str(&html[cursor..pos]);
                let close = format!("</{tag}>");
                match lower[pos..].find(&close) {
                    Some(end) => cursor = pos + end + close.len(),
                    // Truncated body or unterminated block: keep what we already have.
                    None => break,
                }
            }
            None => {
                out.push_str(&html[cursor..]);
                break;
            }
        }
    }
    let spaced = blocks_to_newlines(&out);
    let text = strip_tags(&spaced);
    let text = decode_entities(&text);
    collapse_whitespace(&text)
}

/// Earliest occurrence of `<tag` followed by a tag boundary, searched from `from`.
fn find_open_tag<'t>(hay: &str, from: usize, tags: &[&'t str]) -> Option<(usize, &'t str)> {
    let bytes = hay.as_bytes();
    let mut best: Option<(usize, &str)> = None;
    for tag in tags {
        let needle = format!("<{tag}");
        let mut idx = from;
        while let Some(p) = hay[idx..].find(&needle) {
            let abs = idx + p;
            let boundary = bytes.get(abs + needle.len()).copied();
            if matches!(boundary, Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/')) {
                if best.map(|(bp, _)| abs < bp).unwrap_or(true) {
                    best = Some((abs, tag));
                }
                break;
            }
            idx = abs + needle.len();
        }
    }
    best
}

/// Insert newlines at block-level boundaries so paragraphs survive tag stripping.
fn blocks_to_newlines(input: &str) -> String {
    const BLOCKS: [&str; 17] = [
        "<br", "</p", "</div", "</li", "</tr", "</h1", "</h2", "</h3", "</h4", "</h5", "</h6",
        "</section", "</article", "</td", "</blockquote", "</pre", "</header",
    ];
    let mut out = String::with_capacity(input.len() + 64);
    let mut rest = input;
    while let Some(pos) = BLOCKS.iter().filter_map(|b| rest.find(b)).min() {
        out.push_str(&rest[..pos]);
        out.push('\n');
        match rest[pos..].find('>') {
            Some(gt) => rest = &rest[pos + gt + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(&trimmed);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Char-boundary-safe truncation (never splits a UTF-8 code point).
pub fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n…[truncated]");
    (out, true)
}

pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_ascii_lowercase()))
}

pub fn normalize_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            u.set_fragment(None);
            // strip common tracking params
            let drop: Vec<String> = u
                .query_pairs()
                .filter(|(k, _)| {
                    let k = k.to_ascii_lowercase();
                    k.starts_with("utm_") || k == "fbclid" || k == "gclid" || k == "spm" || k == "ref_src"
                })
                .map(|(k, _)| k.into_owned())
                .collect();
            if !drop.is_empty() {
                let kept: Vec<(String, String)> = u
                    .query_pairs()
                    .filter(|(k, _)| !drop.contains(&k.to_ascii_lowercase()))
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                if kept.is_empty() {
                    u.set_query(None);
                } else {
                    let q = kept
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join("&");
                    u.set_query(Some(&q));
                }
            }
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

/// FNV-1a 64 -> short base36 id, enough to key report entities.
pub fn short_hash(s: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    let mut out = String::new();
    let mut n = hash;
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyz";
    for _ in 0..10 {
        out.push(alphabet[(n % 36) as usize] as char);
        n /= 36;
    }
    out
}

pub fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for ch in s.chars() {
        if ch == '=' || ch == '\n' || ch == '\r' {
            continue;
        }
        let v = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '-' | '+' => 62,
            '_' | '/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// terminal hyperlinks (OSC 8)
// ---------------------------------------------------------------------------

/// Wrap `text` in an OSC 8 hyperlink so a terminal can open `uri` on click.
///
/// `ESC ] 8 ; ; <uri> ESC \ <text> ESC ] 8 ; ; ESC \` — understood by every
/// mainstream terminal today (iTerm2, Ghostty, Kitty, WezTerm, Windows
/// Terminal, GNOME Terminal, VS Code…). Terminals without support ignore the
/// two escape sequences and simply print the visible text.
pub fn hyperlink(uri: &str, text: &str, enabled: bool) -> String {
    if !enabled || uri.trim().is_empty() {
        return text.to_string();
    }
    format!("\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// Should the CLI emit OSC 8 links? Same gate as colour (a TTY, no `NO_COLOR`),
/// with the GNU-style `FORCE_HYPERLINK` overrides: `1` forces it on, anything
/// else (including `0`) forces it off.
pub fn hyperlinks_enabled(tty_with_color: bool) -> bool {
    match std::env::var("FORCE_HYPERLINK") {
        Ok(v) => v == "1",
        Err(_) => tty_with_color,
    }
}

/// A `file://` URL the terminal can hand to the OS opener: made absolute first,
/// with the bytes that would otherwise break URL parsing percent-encoded.
/// Non-ASCII is kept as raw UTF-8, which every modern terminal accepts.
pub fn file_url(path: &std::path::Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut s = String::from("file://");
    for ch in abs.to_string_lossy().chars() {
        match ch {
            '%' => s.push_str("%25"),
            '#' => s.push_str("%23"),
            '?' => s.push_str("%3F"),
            ' ' | '\n' | '\t' | '\r' => s.push_str("%20"),
            c if (c as u32) < 0x20 => s.push_str(&format!("%{:02X}", c as u32)),
            c => s.push(c),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperlink_renders_osc8_only_when_enabled() {
        assert_eq!(hyperlink("https://x.example", "x", false), "x");
        assert_eq!(hyperlink("", "x", true), "x");
        let on = hyperlink("https://x.example", "x", true);
        assert!(on.starts_with("\x1b]8;;https://x.example\x1b\\"), "{on:?}");
        assert!(on.ends_with("\x1b]8;;\x1b\\"), "{on:?}");
        assert!(on.contains("x"));
    }

    #[test]
    fn file_url_is_absolute_and_escaped() {
        let u = file_url(std::path::Path::new("/tmp/a dir/报告 v1.html"));
        assert!(u.starts_with("file:///tmp/a%20dir/"), "{u}");
        assert!(u.contains("v1.html"), "{u}");
        let rel = file_url(std::path::Path::new("visualizations/x.html"));
        assert!(rel.starts_with("file:///"), "relative paths must be resolved: {rel}");
    }

    #[test]
    fn header_tag_must_not_swallow_the_document() {
        let html = "<html><head><title>T</title><script>var x=1;</script><style>.a{}</style></head><body><header>Site nav</header><p>Real content here.</p><p>Second paragraph.</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Real content here."), "{text}");
        assert!(text.contains("Second paragraph."), "{text}");
        assert!(text.contains("Site nav"), "{text}");
        assert!(!text.contains("var x=1"), "script body leaked: {text}");
        assert!(!text.contains(".a{}"), "style body leaked: {text}");
        assert!(!text.contains("here.</p>"), "tags not stripped: {text}");
    }

    #[test]
    fn unterminated_script_keeps_earlier_text() {
        let html = "<body><p>Kept text</p><script>unterminated";
        let text = html_to_text(html);
        assert!(text.contains("Kept text"), "{text}");
    }

    #[test]
    fn iso_round_trip() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_770_091_200), "2026-02-03T04:00:00Z");
    }

    #[test]
    fn local_time_carries_the_utc_offset() {
        let s = iso8601_now_local();
        assert!(s.contains("UTC"), "local timestamp must carry UTC info: {s}");
        assert!(s.len() >= 25, "unexpected local timestamp shape: {s}");
    }

    #[test]
    fn dates_parse() {
        assert_eq!(parse_flexible_date("Tue, 03 Feb 2026 04:00:00 GMT"), Some(1_770_091_200));
        assert_eq!(parse_flexible_date("2026-02-03T04:00:00Z"), Some(1_770_091_200));
        assert_eq!(parse_flexible_date("2026-02-03"), Some(1_770_076_800));
        assert!(parse_flexible_date("not a date").is_none());
    }

    #[test]
    fn entities_and_text() {
        assert_eq!(decode_entities("a &amp; b &#65; &#x42;"), "a & b A B");
        assert_eq!(strip_tags("<b>hi</b>"), "hi");
        let t = html_to_text("<html><head><title>x</title></head><body><p>Hello&nbsp;world</p><script>bad()</script></body></html>");
        assert!(t.contains("Hello world"), "{t}");
        assert!(!t.contains("bad()"), "{t}");
    }

    #[test]
    fn slug_handles_cjk() {
        assert_eq!(slugify("Hello World! 2026"), "hello-world-2026");
        assert_eq!(slugify("验证信息来源"), "验证信息来源");
        assert!(slugify("///").starts_with("report-"));
    }

    #[test]
    fn b64url_decodes_bing_wrapped_url() {
        let enc = "aHR0cHM6Ly9ydXN0LWxhbmcub3JnLw";
        assert_eq!(String::from_utf8(base64url_decode(enc).unwrap()).unwrap(), "https://rust-lang.org/");
    }

    #[test]
    fn truncation_is_char_safe() {
        let s = "中文测试中文测试";
        let (t, trunc) = truncate_chars(s, 4);
        assert!(trunc);
        assert!(t.starts_with("中文测试"));
    }

    #[test]
    fn tracking_params_removed() {
        assert_eq!(normalize_url("https://a.com/x?utm_source=q&id=1#frag"), "https://a.com/x?id=1");
    }
}
