//! Sidebar, file viewer, status bar and modal panels.

use super::*;

pub(super) fn draw_agents(f: &mut Frame, app: &App, area: Rect) {
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

pub(super) fn draw_files(f: &mut Frame, app: &App, area: Rect) {
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

pub(super) fn draw_file_view(f: &mut Frame, app: &App, fv: &FileView, area: Rect) {
    let block = Block::default()
        .title(format!("file: {}", fv.path))
        .borders(Borders::ALL)
        .border_style(border_style(true));
    let commented: Vec<u32> = app
        .view_comment_path()
        .map(|path| {
            app.store
                .comments(&app.agent_key())
                .iter()
                .filter(|c| c.path == path)
                .map(|c| c.line_no)
                .collect()
        })
        .unwrap_or_default();
    let width = area.width.saturating_sub(2) as usize;
    let mut no = 0usize;
    let mut lines: Vec<Line> = Vec::new();
    for (i, fl) in fv.lines.iter().enumerate() {
        let cursor_here = i == app.fv_cursor && app.right == RightPane::File;
        let line = match fl {
            FvLine::Content { line, changed } => {
                no += 1;
                let gutter = if commented.contains(&(no as u32)) {
                    Span::styled("●", Style::default().fg(Color::Yellow))
                } else if *changed {
                    Span::styled("▎", Style::default().fg(Color::Green))
                } else {
                    Span::raw(" ")
                };
                let mut spans =
                    vec![gutter, Span::styled(format!("{no:>5} "), Style::default().fg(Color::DarkGray))];
                spans.extend(slice_spans(&line.spans, app.hscroll as usize));
                if cursor_here {
                    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                    if used < width {
                        spans.push(Span::raw(" ".repeat(width - used)));
                    }
                    Line::from(spans).style(Style::default().bg(CUR_CTX_BG))
                } else {
                    Line::from(spans)
                }
            }
            FvLine::Deleted { text } => {
                let span = Span::styled(format!("- {text}"), Style::default().fg(Color::Red));
                Line::from(vec![Span::styled("      ", Style::default()), span])
            }
        };
        lines.push(line);
        // inline comment input right under the viewer cursor
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
    f.render_widget(Paragraph::new(lines).scroll((app.fv_scroll, 0)).block(block), area);
}

pub(super) fn draw_status(f: &mut Frame, app: &App, area: Rect) {
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

pub(super) fn draw_confirm(f: &mut Frame, screen: Rect) {
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
