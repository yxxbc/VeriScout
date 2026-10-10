//! Terminal rendering of the model's Markdown answer.
//!
//! The report used to be where the model's prose was read, so Markdown fidelity only
//! mattered to the HTML renderer. Now the terminal prints the answer itself, and raw
//! `##` / `**` as the first thing a user sees is worse than the plain text it
//! replaced. No parser lives here: this is a thin policy layer over `termimad`,
//! picking the skin (styled when colours are on, structure-only text otherwise) and
//! the wrap width (the terminal's own width on a TTY, effectively unbounded when the
//! output is piped somewhere).

use termimad::MadSkin;

/// Width used when there is no terminal worth measuring (pipes, `--no-color`,
/// `--json`): wide enough that nothing is re-wrapped, small enough to stay clear of
/// overflow arithmetic inside the renderer.
const PIPE_WIDTH: usize = 10_000;

/// Render Markdown for a terminal.
///
/// With `color = false` the result is plain text: no escape sequences at all, with
/// the document structure (bullets, numbers, code indentation) preserved and the
/// syntax removed.
pub fn render(text: &str, color: bool) -> String {
    let rendered = if color {
        // `term_text` measures the terminal and wraps to it.
        MadSkin::default_dark().term_text(text).to_string()
    } else {
        MadSkin::no_style().text(text, Some(PIPE_WIDTH)).to_string()
    };
    rendered.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESC: char = '\x1b';
    const SAMPLE: &str = "# 结论\n\n一次性纸杯**可以**回收，但要看分类。\n\n## 要点\n\n- 多数城市要求撕掉杯盖\n- 内层淋膜不是纸浆：[指南](https://example.com/guide)\n\n1. 检查杯盖\n2. 投纸张回收桶\n\n> 有争议：部分回收站拒收\n\n```python\n## 不是标题\nx = 1\n```\n\n| 城市 | 结果 |\n| --- | --- |\n| 上海 | 可回收 |";

    #[test]
    fn plain_output_has_no_escape_sequences_but_keeps_structure() {
        let out = render(SAMPLE, false);
        assert!(!out.contains(ESC), "no colours without a colour terminal: {out:?}");
        assert!(!out.contains("**"), "emphasis markers must be consumed: {out:?}");
        assert!(out.contains("结论"), "heading text must survive");
        assert!(out.contains("• 多数城市要求撕掉杯盖"), "bullets must be rendered: {out:?}");
        assert!(out.contains("1. 检查杯盖"), "ordered items must keep their number: {out:?}");
        assert!(out.contains("## 不是标题"), "code fences are not markdown: {out:?}");
        assert!(out.contains("https://example.com/guide"), "links must stay visible: {out:?}");
    }

    #[test]
    fn coloured_output_uses_ansi_and_keeps_the_text() {
        let out = render(SAMPLE, true);
        assert!(out.contains(ESC), "colour mode must style the text");
        for needle in ["结论", "要点", "一次性纸杯", "上海"] {
            assert!(out.contains(needle), "{needle} missing from {out:?}");
        }
        assert!(!out.contains("## 结论"), "ATX markers must not leak: {out:?}");
    }

    #[test]
    fn empty_input_renders_empty() {
        assert_eq!(render("", false), "");
        assert_eq!(render("   \n\n", false), "");
    }

    #[test]
    fn plain_output_does_not_wrap_long_lines() {
        let long = format!("{}。", "这是一句很长的说明".repeat(20));
        let out = render(&long, false);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 1, "piped output must not be wrapped: {lines:?}");
        assert!(lines[0].chars().count() > 100);
    }
}
