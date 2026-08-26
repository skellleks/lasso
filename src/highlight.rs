//! Syntect-based file highlighting for the viewer pane.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    static TS: OnceLock<Theme> = OnceLock::new();
    TS.get_or_init(|| ThemeSet::load_defaults().themes["base16-ocean.dark"].clone())
}

/// Syntax-highlight a single line of `path`'s language (for diff rendering).
pub fn highlight_line(path: &str, text: &str) -> Vec<Span<'static>> {
    let ss = syntax_set();
    let Some(syntax) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| ss.find_syntax_by_extension(ext))
    else {
        return vec![Span::raw(text.to_string())];
    };
    let mut hl = HighlightLines::new(syntax, theme());
    match hl.highlight_line(text, ss) {
        Ok(ranges) => ranges
            .into_iter()
            .map(|(style, chunk)| Span::styled(chunk.to_string(), to_ratatui(style)))
            .filter(|s| !s.content.is_empty())
            .collect(),
        Err(_) => vec![Span::raw(text.to_string())],
    }
}

/// Highlight `content`, marking line numbers in `changed` (1-based).
pub fn highlight_file(path: &str, content: &str, changed: &BTreeSet<u32>) -> Vec<(Line<'static>, bool)> {
    let ss = syntax_set();
    let syntax = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| ss.find_syntax_by_extension(ext))
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut hl = HighlightLines::new(syntax, theme());

    LinesWithEndings::from(content)
        .enumerate()
        .map(|(i, line)| {
            let spans: Vec<Span<'static>> = match hl.highlight_line(line, ss) {
                Ok(ranges) => ranges
                    .into_iter()
                    .map(|(style, text)| {
                        Span::styled(text.trim_end_matches('\n').to_string(), to_ratatui(style))
                    })
                    .filter(|s| !s.content.is_empty())
                    .collect(),
                Err(_) => vec![Span::raw(line.trim_end_matches('\n').to_string())],
            };
            (Line::from(spans), changed.contains(&(i as u32 + 1)))
        })
        .collect()
}

fn to_ratatui(style: SynStyle) -> Style {
    let fg = style.foreground;
    let (r, g, b) = boost(fg.r, fg.g, fg.b);
    Style::default().fg(Color::Rgb(r, g, b))
}

/// Push saturation and brightness up a bit — the base16 palette reads muted
/// on dark terminal backgrounds.
fn boost(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let avg = (r + g + b) / 3.0;
    let sat = |c: f32| (avg + (c - avg) * 1.6).clamp(0.0, 255.0);
    let bright = |c: f32| (c * 1.12).clamp(0.0, 255.0);
    (bright(sat(r)) as u8, bright(sat(g)) as u8, bright(sat(b)) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[(Line<'static>, bool)]) -> Vec<String> {
        lines
            .iter()
            .map(|(l, _)| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect()
    }

    #[test]
    fn keeps_text_and_marks_changed_lines() {
        let changed = BTreeSet::from([2]);
        let out = highlight_file("x.py", "import os\nprint(1)\n", &changed);
        assert_eq!(texts(&out), vec!["import os", "print(1)"]);
        assert!(!out[0].1);
        assert!(out[1].1);
    }

    #[test]
    fn python_line_gets_multiple_style_spans() {
        let out = highlight_file("x.py", "def f(x):\n", &BTreeSet::new());
        assert!(out[0].0.spans.len() > 1, "syntax highlighting produced styled spans");
    }

    #[test]
    fn unknown_extension_falls_back_to_plain() {
        let out = highlight_file("data.xyz123", "hello world\n", &BTreeSet::new());
        assert_eq!(texts(&out), vec!["hello world"]);
    }

    #[test]
    fn empty_file_is_empty() {
        assert!(highlight_file("a.rs", "", &BTreeSet::new()).is_empty());
    }

    #[test]
    fn boost_saturates_muted_colors_but_keeps_grays() {
        // muted green from base16: spread between channels must grow
        let (r, g, b) = boost(163, 190, 140);
        assert!(g as i32 - r as i32 > 190 - 163, "more saturated: {r},{g},{b}");
        assert!(g >= 190, "not darker");
        // pure gray stays gray
        let (r, g, b) = boost(128, 128, 128);
        assert_eq!((r, g), (g, b));
    }

    #[test]
    fn highlight_line_styles_code_and_keeps_text() {
        let spans = highlight_line("x.rs", "fn main() {");
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "fn main() {");
        assert!(spans.len() > 1, "styled into multiple spans");
        // unknown extension: plain single span
        let plain = highlight_line("x.zzz", "hello");
        let text: String = plain.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hello");
    }
}
