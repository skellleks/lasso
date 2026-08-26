//! Rendering. Pure: App (+ prepared file view) → ratatui frame.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus, Modal, RightPane, Row};
use crate::diff::LineKind;
use crate::review::Side;

/// One row of the file viewer.
pub enum FvLine {
    /// A line of the current file content, syntax-styled.
    Content { line: Line<'static>, changed: bool },
    /// A line deleted by the diff, shown inline where it used to be.
    Deleted { text: String },
}

/// Pre-highlighted file for the viewer pane.
#[derive(Default)]
pub struct FileView {
    pub path: String,
    pub lines: Vec<FvLine>,
}

/// Overlay `bg` onto the given char ranges of styled spans, splitting spans
/// at range boundaries; fg styling is preserved.
pub fn apply_bg_ranges(
    spans: &[Span<'static>],
    ranges: &[(usize, usize)],
    bg: Color,
) -> Vec<Span<'static>> {
    let in_range = |i: usize| ranges.iter().any(|&(a, b)| a <= i && i < b);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut pos = 0usize;
    for span in spans {
        let mut buf = String::new();
        let mut buf_marked = false;
        for ch in span.content.chars() {
            let marked = in_range(pos);
            if !buf.is_empty() && marked != buf_marked {
                let style = if buf_marked { span.style.bg(bg) } else { span.style };
                out.push(Span::styled(std::mem::take(&mut buf), style));
            }
            buf_marked = marked;
            buf.push(ch);
            pos += 1;
        }
        if !buf.is_empty() {
            let style = if buf_marked { span.style.bg(bg) } else { span.style };
            out.push(Span::styled(buf, style));
        }
    }
    out
}

/// Skip the first `skip` characters of a styled line (for horizontal scroll).
pub fn slice_spans(spans: &[Span<'static>], skip: usize) -> Vec<Span<'static>> {
    let mut remaining = skip;
    let mut out = Vec::new();
    for span in spans {
        let len = span.content.chars().count();
        if remaining >= len {
            remaining -= len;
            continue;
        }
        if remaining == 0 {
            out.push(span.clone());
        } else {
            let text: String = span.content.chars().skip(remaining).collect();
            out.push(Span::styled(text, span.style));
            remaining = 0;
        }
    }
    out
}

/// The screen regions used both for drawing and mouse hit-testing.
pub fn regions(area: Rect) -> crate::app::Regions {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(30)])
        .split(outer[0]);
    let sidebar = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(cols[0]);
    crate::app::Regions { agents: sidebar[0], files: sidebar[1], right: cols[1] }
}

pub fn draw(f: &mut Frame, app: &App, file_view: &FileView) {
    let area = f.area();
    let r = regions(area);
    let status = Rect { x: area.x, y: area.y + area.height.saturating_sub(1), width: area.width, height: 1 };

    draw_agents(f, app, r.agents);
    draw_files(f, app, r.files);
    match app.right {
        RightPane::Diff => draw_diff(f, app, r.right),
        RightPane::File => draw_file_view(f, app, file_view, r.right),
    }
    draw_status(f, app, status);
    if matches!(app.modal, Some(Modal::ConfirmSubmit)) {
        draw_confirm(f, area);
    }
}

/// Fills behind added/deleted diff lines, plus brighter cursor variants.
pub(super) const ADD_BG: Color = Color::Rgb(6, 43, 3);
pub(super) const DEL_BG: Color = Color::Rgb(62, 3, 1);
pub(super) const CUR_ADD_BG: Color = Color::Rgb(14, 68, 8);
pub(super) const CUR_DEL_BG: Color = Color::Rgb(92, 10, 6);
pub(super) const CUR_CTX_BG: Color = Color::Rgb(67, 76, 94);
/// Brighter fills for the exact tokens that changed within a line pair.
pub(super) const ADD_EM_BG: Color = Color::Rgb(24, 110, 18);
pub(super) const DEL_EM_BG: Color = Color::Rgb(150, 24, 16);
pub(super) fn border_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

pub(super) fn status_badge(status: &str) -> Span<'static> {
    let (sym, color) = match status {
        "idle" => ("●", Color::Green),
        "working" => ("●", Color::Yellow),
        "blocked" => ("●", Color::Red),
        "done" => ("●", Color::Blue),
        _ => ("○", Color::DarkGray),
    };
    Span::styled(sym.to_string(), Style::default().fg(color))
}

pub(super) fn file_badge(app: &App, path: &str) -> Span<'static> {
    match app.file_index_by_display(path).and_then(|i| app.files.get(i)).map(|f| &f.status) {
        Some(crate::diff::FileStatus::Added) => Span::styled("A", Style::default().fg(Color::Green)),
        Some(crate::diff::FileStatus::Deleted) => Span::styled("D", Style::default().fg(Color::Red)),
        Some(crate::diff::FileStatus::Renamed) => Span::styled("R", Style::default().fg(Color::Magenta)),
        Some(crate::diff::FileStatus::Binary) => Span::styled("B", Style::default().fg(Color::DarkGray)),
        Some(crate::diff::FileStatus::Modified) => Span::styled("M", Style::default().fg(Color::Yellow)),
        None => Span::raw(" "),
    }
}

mod diff;
mod panels;

use diff::draw_diff;
#[cfg(test)]
use diff::emphasis_for;
use panels::{draw_agents, draw_confirm, draw_file_view, draw_files, draw_status};

#[cfg(test)]
mod tests;
