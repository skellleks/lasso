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

/// Char-range emphasis for paired del/add rows: row index → ranges.
fn emphasis_for(rows: &[Row]) -> std::collections::HashMap<usize, Vec<(usize, usize)>> {
    let mut map = std::collections::HashMap::new();
    let kind_of = |r: &Row| match r {
        Row::Line(l) => Some(l.kind.clone()),
        Row::HunkHeader(_) => None,
    };
    let mut i = 0;
    while i < rows.len() {
        if kind_of(&rows[i]) != Some(LineKind::Del) {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < rows.len() && kind_of(&rows[i]) == Some(LineKind::Del) {
            i += 1;
        }
        let add_start = i;
        while i < rows.len() && kind_of(&rows[i]) == Some(LineKind::Add) {
            i += 1;
        }
        let pairs = (i - add_start).min(add_start - del_start);
        for k in 0..pairs {
            let (Row::Line(d), Row::Line(a)) = (&rows[del_start + k], &rows[add_start + k]) else {
                continue;
            };
            let (old_r, new_r) = crate::diff::word_diff_ranges(&d.text, &a.text);
            if !old_r.is_empty() {
                map.insert(del_start + k, old_r);
            }
            if !new_r.is_empty() {
                map.insert(add_start + k, new_r);
            }
        }
    }
    map
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
const ADD_BG: Color = Color::Rgb(6, 43, 3);
const DEL_BG: Color = Color::Rgb(62, 3, 1);
const CUR_ADD_BG: Color = Color::Rgb(14, 68, 8);
const CUR_DEL_BG: Color = Color::Rgb(92, 10, 6);
const CUR_CTX_BG: Color = Color::Rgb(38, 44, 56);
/// Brighter fills for the exact tokens that changed within a line pair.
const ADD_EM_BG: Color = Color::Rgb(24, 110, 18);
const DEL_EM_BG: Color = Color::Rgb(150, 24, 16);

fn border_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn status_badge(status: &str) -> Span<'static> {
    let (sym, color) = match status {
        "idle" => ("●", Color::Green),
        "working" => ("●", Color::Yellow),
        "blocked" => ("●", Color::Red),
        "done" => ("●", Color::Blue),
        _ => ("○", Color::DarkGray),
    };
    Span::styled(sym.to_string(), Style::default().fg(color))
}

fn draw_agents(f: &mut Frame, app: &App, area: Rect) {
    let line = match (&app.agent, app.standalone) {
        (Some(a), _) => Line::from(vec![
            status_badge(&a.agent_status),
            Span::raw(" "),
            Span::styled(a.label().to_string(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", a.agent_status), Style::default().fg(Color::DarkGray)),
        ]),
        (None, true) => Line::from(Span::styled("standalone", Style::default().fg(Color::DarkGray))),
        (None, false) => Line::from(Span::styled("no agent", Style::default().fg(Color::DarkGray))),
    };
    let block = Block::default()
        .title("Agent")
        .borders(Borders::ALL)
        .border_style(border_style(false));
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn file_badge(app: &App, path: &str) -> Span<'static> {
    match app.files.iter().find(|f| f.new_path == path).map(|f| &f.status) {
        Some(crate::diff::FileStatus::Added) => Span::styled("A", Style::default().fg(Color::Green)),
        Some(crate::diff::FileStatus::Deleted) => Span::styled("D", Style::default().fg(Color::Red)),
        Some(crate::diff::FileStatus::Renamed) => Span::styled("R", Style::default().fg(Color::Magenta)),
        Some(crate::diff::FileStatus::Binary) => Span::styled("B", Style::default().fg(Color::DarkGray)),
        Some(crate::diff::FileStatus::Modified) => Span::styled("M", Style::default().fg(Color::Yellow)),
        None => Span::raw(" "),
    }
}

fn draw_files(f: &mut Frame, app: &App, area: Rect) {
    let height = area.height.saturating_sub(2) as usize;
    let rows = app.files_tree_rows();
    let skip = app.files_scroll.min(rows.len().saturating_sub(height.max(1)));
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(i, r)| {
            let selected = i == app.selected_file;
            let indent = "  ".repeat(r.depth);
            let mut style = if r.is_dir {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };
            if selected {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if r.is_dir {
                let arrow = if r.expanded { "▾" } else { "▸" };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("{indent}{arrow} {}", r.name), style),
                ]))
            } else {
                ListItem::new(Line::from(vec![
                    file_badge(app, &r.path),
                    Span::raw(" "),
                    Span::styled(format!("{indent}{}", r.name), style),
                ]))
            }
        })
        .collect();
    let title = if app.all_files_mode { "Files (all)" } else { "Files" };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style(app.focus == Focus::Files));
    f.render_widget(List::new(items).block(block), area);
}

fn draw_diff(f: &mut Frame, app: &App, area: Rect) {
    let rows = app.rows();
    let title = app
        .files
        .get(app.diff_file)
        .map(|fd| format!("diff: {}", fd.new_path))
        .unwrap_or_else(|| "diff".to_string());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style(app.focus == Focus::Diff));

    if rows.is_empty() {
        let msg = Paragraph::new(Span::styled("clean — no changes", Style::default().fg(Color::DarkGray)))
            .block(block);
        f.render_widget(msg, area);
        return;
    }

    let commented = commented_lines(app);
    let emphasis = emphasis_for(&rows);
    let height = area.height.saturating_sub(2) as usize;
    let top = app.diff_scroll.min(rows.len().saturating_sub(height.max(1)));
    let path = app.files.get(app.diff_file).map(|f| f.new_path.clone()).unwrap_or_default();
    let width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(top).take(height.max(1)) {
        let cursor_here = i == app.cursor && app.focus == Focus::Diff;
        let line = match row {
            Row::HunkHeader(h) => {
                let mut style = Style::default().fg(Color::Cyan);
                if cursor_here {
                    style = style.bg(CUR_CTX_BG);
                }
                Line::from(Span::styled(format!("        {h}"), style))
            }
            Row::Line(l) => {
                let (sign, sign_color, bg) = match (&l.kind, cursor_here) {
                    (LineKind::Add, false) => ("+", Color::Green, Some(ADD_BG)),
                    (LineKind::Add, true) => ("+", Color::Green, Some(CUR_ADD_BG)),
                    (LineKind::Del, false) => ("-", Color::Red, Some(DEL_BG)),
                    (LineKind::Del, true) => ("-", Color::Red, Some(CUR_DEL_BG)),
                    (LineKind::Context, false) => (" ", Color::DarkGray, None),
                    (LineKind::Context, true) => (" ", Color::DarkGray, Some(CUR_CTX_BG)),
                };
                let no = l.new_no.or(l.old_no).unwrap_or(0);
                let side = if l.kind == LineKind::Del { Side::Old } else { Side::New };
                let mark = if commented.contains(&(side, no)) { "●" } else { " " };

                let mut text_spans = crate::highlight::highlight_line(&path, &l.text);
                if let Some(ranges) = emphasis.get(&i) {
                    let em = if l.kind == LineKind::Add { ADD_EM_BG } else { DEL_EM_BG };
                    text_spans = apply_bg_ranges(&text_spans, ranges, em);
                }
                let mut content = vec![Span::styled(sign.to_string(), Style::default().fg(sign_color))];
                content.extend(text_spans);
                let mut spans =
                    vec![Span::styled(format!("{mark}{no:>5} "), Style::default().fg(Color::DarkGray))];
                spans.extend(slice_spans(&content, app.hscroll as usize));
                // fill the whole row so add/del lines read as colored bands
                let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                if used < width {
                    spans.push(Span::raw(" ".repeat(width - used)));
                }
                let mut line = Line::from(spans);
                if let Some(bg) = bg {
                    line = line.style(Style::default().bg(bg));
                }
                line
            }
        };
        lines.push(line);
        // inline comment input right under the line being commented
        if cursor_here {
            if let Some(Modal::Input { buffer }) = &app.modal {
                lines.push(Line::from(vec![
                    Span::styled("       └ ", Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!("{buffer}▏"),
                        Style::default().fg(Color::Black).bg(Color::Yellow),
                    ),
                    Span::styled("  (Enter to save, Esc to cancel)", Style::default().fg(Color::DarkGray)),
                ]));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn commented_lines(app: &App) -> Vec<(Side, u32)> {
    let path = app.files.get(app.diff_file).map(|f| f.new_path.as_str()).unwrap_or("");
    app.store
        .comments(&app.agent_key())
        .iter()
        .filter(|c| c.path == path)
        .map(|c| (c.side.clone(), c.line_no))
        .collect()
}

fn draw_file_view(f: &mut Frame, app: &App, fv: &FileView, area: Rect) {
    let block = Block::default()
        .title(format!("file: {}", fv.path))
        .borders(Borders::ALL)
        .border_style(border_style(true));
    let mut no = 0usize;
    let lines: Vec<Line> = fv
        .lines
        .iter()
        .map(|fl| match fl {
            FvLine::Content { line, changed } => {
                no += 1;
                let gutter = if *changed {
                    Span::styled("▎", Style::default().fg(Color::Green))
                } else {
                    Span::raw(" ")
                };
                let mut spans =
                    vec![gutter, Span::styled(format!("{no:>5} "), Style::default().fg(Color::DarkGray))];
                spans.extend(slice_spans(&line.spans, app.hscroll as usize));
                Line::from(spans)
            }
            FvLine::Deleted { text } => {
                let span = Span::styled(format!("- {text}"), Style::default().fg(Color::Red));
                Line::from(vec![
                    Span::styled("      ", Style::default()),
                    span,
                ])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines).scroll((app.fv_scroll, 0)).block(block), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    const KEY: Style = Style::new().fg(Color::Rgb(136, 192, 208)).bg(Color::Rgb(46, 52, 64));
    const LABEL: Style = Style::new().fg(Color::Rgb(97, 110, 136));
    const SEP: Style = Style::new().fg(Color::Rgb(59, 66, 82));

    // (key, label, drop priority: higher drops first; 0 never drops)
    let hints: &[(&str, &str, u8)] = &[
        ("j/k h/l", "nav", 0),
        ("⇥", "focus", 6),
        ("↵", "view", 5),
        ("c", "comment", 0),
        ("x", "del", 4),
        ("S", "submit", 0),
        ("b", "base", 2),
        ("a", "all", 3),
        ("r", "refresh", 1),
        ("q", "quit", 0),
    ];
    // shrink by dropping hints highest-priority-first, never the labels
    let build = |hints: &[(&str, &str, u8)], seps: bool| -> Vec<Span> {
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        for (i, (key, label, _)) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(if seps { "  ·  " } else { " " }, SEP));
            }
            spans.push(Span::styled(format!(" {key} "), KEY));
            spans.push(Span::styled(format!(" {label}"), LABEL));
        }
        spans
    };
    let width = |spans: &[Span]| -> usize { spans.iter().map(|s| s.content.chars().count()).sum() };
    let mut visible: Vec<(&str, &str, u8)> = hints.to_vec();
    let mut spans = build(&visible, true);
    if width(&spans) > area.width as usize {
        spans = build(&visible, false);
        while width(&spans) > area.width as usize {
            let Some(pos) = visible
                .iter()
                .enumerate()
                .filter(|(_, h)| h.2 > 0)
                .max_by_key(|(_, h)| h.2)
                .map(|(i, _)| i)
            else {
                break;
            };
            visible.remove(pos);
            spans = build(&visible, false);
        }
    }

    // right side is computed below; stretch the hint gaps to fill what's left

    // right side: state badges
    let mut right: Vec<Span> = Vec::new();
    if app.standalone {
        right.push(Span::styled(" standalone ", Style::default().fg(Color::Black).bg(Color::Yellow)));
        right.push(Span::raw(" "));
    }
    let count = app.store.comments(&app.agent_key()).len();
    if count > 0 {
        right.push(Span::styled(
            format!(" ✎ {count} "),
            Style::default().fg(Color::Black).bg(Color::Rgb(235, 203, 139)),
        ));
        right.push(Span::raw(" "));
    }
    if !app.status.is_empty() {
        right.push(Span::styled(
            format!(" {} ", app.status),
            Style::default().fg(Color::Rgb(136, 192, 208)),
        ));
    }

    let left_w = width(&spans);
    let right_w: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let total = area.width as usize;
    if left_w + right_w < total {
        // distribute the leftover space evenly into the gaps between hints
        let seps: Vec<usize> = spans
            .iter()
            .enumerate()
            .filter(|(_, s)| s.style == SEP || s.content.chars().all(|c| c == ' '))
            .map(|(i, _)| i)
            .filter(|&i| i > 0)
            .collect();
        let mut extra = total - left_w - right_w;
        if !seps.is_empty() {
            let per = extra / seps.len();
            let mut rem = extra % seps.len();
            for &i in &seps {
                let add = per + if rem > 0 { rem -= 1; 1 } else { 0 };
                let widened = format!("{}{}", spans[i].content, " ".repeat(add));
                spans[i] = Span::styled(widened, spans[i].style);
            }
            extra = 0;
        }
        if extra > 0 {
            spans.push(Span::raw(" ".repeat(extra)));
        }
    }
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_confirm(f: &mut Frame, screen: Rect) {
    let (title, text) = (
        "Agent is busy",
        "Agent is not idle. Send review anyway? (y/n)".to_string(),
    );
    let w = screen.width.saturating_sub(10).clamp(20, 80);
    let area = Rect {
        x: screen.x + (screen.width.saturating_sub(w)) / 2,
        y: screen.y + screen.height / 2 - 2,
        width: w,
        height: 4,
    };
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    f.render_widget(Paragraph::new(text).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::parse;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const DIFF: &str = "\
diff --git a/user.py b/user.py
index 1111111..2222222 100644
--- a/user.py
+++ b/user.py
@@ -41,3 +41,3 @@
 def get_user(id):
-    return None
+    return user
";

    fn render(app: &App, fv: &FileView) -> String {
        let backend = TestBackend::new(100, 24);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, app, fv)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn test_app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::new(dir.path(), false);
        a.pin("w1:p1".to_string());
        a.set_agents(vec![serde_json::from_value(serde_json::json!({
            "pane_id": "w1:p1", "agent": "claude", "agent_status": "working",
            "cwd": "/repo/api", "terminal_title_stripped": "fix login"
        }))
        .unwrap()]);
        a.set_diff(parse(DIFF));
        a
    }

    #[test]
    fn draws_agents_files_and_diff() {
        let screen = render(&test_app(), &FileView::default());
        assert!(screen.contains("api"), "agent label visible");
        assert!(screen.contains("user.py"), "file listed");
        assert!(screen.contains("return user"), "diff line visible");
        assert!(screen.contains("@@ -41,3 +41,3 @@"), "hunk header visible");
    }

    #[test]
    fn draws_comment_input_modal() {
        let mut app = test_app();
        app.focus = Focus::Diff;
        app.cursor = 3;
        app.handle_key('c');
        app.handle_key('h');
        app.handle_key('i');
        let screen = render(&app, &FileView::default());
        assert!(screen.contains("hi"), "typed text visible");
        assert!(screen.contains("└"), "inline input marker visible");
    }

    #[test]
    fn draws_file_view_with_inline_deletions() {
        let mut app = test_app();
        app.right = RightPane::File;
        let fv = FileView {
            path: "user.py".into(),
            lines: vec![
                FvLine::Content { line: Line::from("import os"), changed: false },
                FvLine::Deleted { text: "removed_line = 0".into() },
                FvLine::Content { line: Line::from("changed_line = 1"), changed: true },
            ],
        };
        let screen = render(&app, &fv);
        let lines: Vec<&str> = screen.lines().collect();
        let import_row = lines.iter().position(|l| l.contains("import os")).unwrap();
        assert!(lines[import_row].contains("    1 "), "content numbered");
        assert!(lines[import_row + 1].contains("- removed_line = 0"), "deleted inline");
        assert!(lines[import_row + 2].contains("    2 "), "numbering skips deleted rows");
        assert!(lines[import_row + 2].contains("changed_line = 1"));
    }

    #[test]
    fn draws_all_files_tree_with_change_badges() {
        let mut app = test_app();
        app.all_files_mode = true;
        app.all_files = ["src/lib.rs", "user.py", "README.md"].iter().map(|s| s.to_string()).collect();
        let screen = render(&app, &FileView::default());
        assert!(screen.contains("▾ src"), "expanded dir with arrow");
        assert!(screen.contains("  lib.rs"), "indented child");
        // user.py is modified in the diff → badge
        let badge_row = screen
            .lines()
            .map(|l| l.chars().take(26).collect::<String>())
            .find(|l| l.contains("user.py"))
            .unwrap();
        assert!(badge_row.contains('M'), "changed file marked: {badge_row}");
        app.collapsed.insert("src".into());
        let screen = render(&app, &FileView::default());
        assert!(screen.contains("▸ src"), "collapsed arrow");
        assert!(!screen.contains("lib.rs"), "children hidden");
    }

    #[test]
    fn slice_spans_skips_across_span_boundaries() {
        let spans = vec![Span::raw("héllo"), Span::raw(" wörld")]; // non-ASCII: exercises char-safe slicing
        let out = slice_spans(&spans, 4);
        let text: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "o wörld");
        let out = slice_spans(&spans, 0);
        let text: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "héllo wörld");
        assert!(slice_spans(&spans, 99).is_empty());
    }

    #[test]
    fn comment_input_renders_inline_under_cursor_line() {
        let mut app = test_app();
        app.focus = Focus::Diff;
        app.cursor = 3; // "+    return user"
        app.handle_key('c');
        app.handle_key('h');
        app.handle_key('i');
        let screen = render(&app, &FileView::default());
        let lines: Vec<&str> = screen.lines().collect();
        let code_row = lines.iter().position(|l| l.contains("return user")).unwrap();
        assert!(
            lines[code_row + 1].contains("hi"),
            "input right under the commented line:\n{screen}"
        );
        assert!(!screen.contains("Comment (Enter"), "no centered popup for input");
    }

    #[test]
    fn diff_lines_have_add_del_fill_and_syntax_spans() {
        let app = test_app(); // diff of user.py
        let backend = TestBackend::new(100, 24);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, &app, &FileView::default())).unwrap();
        let buf = term.backend().buffer().clone();
        let mut add_bg = None;
        let mut del_bg = None;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect();
            if line.contains("return user") {
                let x = line.find("return user").unwrap() as u16;
                add_bg = Some(buf[(x, y)].style().bg);
                // fill continues past end of text
                let tail = buf[(buf.area.width - 2, y)].style().bg;
                assert_eq!(tail, add_bg.unwrap(), "fill spans the whole row");
            }
            if line.contains("return None") {
                let x = line.find("return None").unwrap() as u16;
                del_bg = Some(buf[(x, y)].style().bg);
            }
        }
        let (add_bg, del_bg) = (add_bg.expect("add line"), del_bg.expect("del line"));
        assert_ne!(add_bg, del_bg, "green vs red fill");
        assert!(add_bg.is_some() && del_bg.is_some(), "both lines filled");
    }

    #[test]
    fn apply_bg_ranges_splits_spans_and_sets_bg() {
        let spans = vec![Span::styled("abc", Style::default().fg(Color::Red)), Span::raw("defg")];
        let out = apply_bg_ranges(&spans, &[(2, 5)], Color::Blue);
        let text: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "abcdefg", "text unchanged");
        for (i, ch) in text.chars().enumerate() {
            let mut pos = 0;
            for s in &out {
                let len = s.content.chars().count();
                if i < pos + len {
                    let expect = (2..5).contains(&i);
                    assert_eq!(s.style.bg == Some(Color::Blue), expect, "char {ch} at {i}");
                    if i < 3 {
                        assert_eq!(s.style.fg, Some(Color::Red), "fg preserved");
                    }
                    break;
                }
                pos += len;
            }
        }
    }

    #[test]
    fn emphasis_pairs_del_and_add_runs() {
        let rows = crate::diff::parse(DIFF)
            .remove(0)
            .hunks
            .remove(0)
            .lines
            .into_iter()
            .map(Row::Line)
            .collect::<Vec<_>>();
        // rows: ctx "def get_user(id):", del "    return None", add "    return user"
        let map = emphasis_for(&rows);
        assert!(map.get(&1).is_some_and(|r| !r.is_empty()), "del row emphasized: {map:?}");
        assert!(map.get(&2).is_some_and(|r| !r.is_empty()), "add row emphasized");
        assert!(!map.contains_key(&0), "context untouched");
    }

    #[test]
    fn cursor_line_uses_background_band_not_reversed() {
        let mut app = test_app();
        app.focus = Focus::Diff;
        app.cursor = 3; // "+    return user" (hunk fallback rows)
        let backend = TestBackend::new(100, 24);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, &app, &FileView::default())).unwrap();
        let buf = term.backend().buffer().clone();
        for y in 0..buf.area.height {
            let line: String =
                (0..buf.area.width).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect();
            if line.contains("return user") {
                let x = line.find("return user").unwrap() as u16;
                let style = buf[(x, y)].style();
                assert!(
                    !style.add_modifier.contains(ratatui::style::Modifier::REVERSED),
                    "no reversed video on the cursor line"
                );
                assert_eq!(style.bg, Some(CUR_ADD_BG), "cursor uses a brighter add band");
                return;
            }
        }
        panic!("cursor line not found");
    }

    #[test]
    fn diff_hscroll_shifts_text_but_not_gutter() {
        let mut app = test_app();
        app.focus = Focus::Diff;
        app.hscroll = 8;
        let screen = render(&app, &FileView::default());
        assert!(screen.contains("   41 "), "line numbers intact");
        assert!(!screen.contains("def get_user"), "text shifted by 8 chars");
        assert!(screen.contains("_user(id):"), "shifted tail visible");
    }

    #[test]
    fn status_bar_renders_key_chips() {
        let app = test_app();
        let backend = TestBackend::new(120, 24);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, &app, &FileView::default())).unwrap();
        let buf = term.backend().buffer().clone();
        let y = buf.area.height - 1;
        let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect();
        assert!(line.contains("nav"), "labels present: {line}");
        assert!(line.contains("submit"));
        // the key part is styled differently from its label
        let kx = line.find("j/k").unwrap() as u16;
        let lx = line.find("nav").unwrap() as u16;
        assert_ne!(buf[(kx, y)].style().fg, buf[(lx, y)].style().fg, "key chip differs from label");
    }

    #[test]
    fn status_bar_fits_narrow_panes() {
        let app = test_app();
        for width in [50u16, 70, 90] {
            let backend = TestBackend::new(width, 24);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| draw(f, &app, &FileView::default())).unwrap();
            let buf = term.backend().buffer().clone();
            let y = buf.area.height - 1;
            let line: String =
                (0..buf.area.width).map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' ')).collect();
            assert!(line.contains("quit"), "quit labeled at width {width}: {line}");
            assert!(line.contains("submit"), "labels never dropped at width {width}: {line}");
            if width == 90 {
                assert!(line.contains("refresh"), "medium width keeps secondary hints: {line}");
            }
            let end = line.rfind("quit").unwrap() + 4;
            assert!(
                end as u16 >= width.saturating_sub(3),
                "hints fill the width at {width}: end={end}, line: {line}"
            );
        }
    }

    #[test]
    fn empty_app_renders_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(dir.path(), true);
        let screen = render(&app, &FileView::default());
        assert!(screen.contains("clean") || screen.contains("no changes"), "placeholder shown");
    }
}
