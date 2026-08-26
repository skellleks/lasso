//! The diff pane: whole-file rendering with word-level emphasis.

use super::*;

/// Char-range emphasis for paired del/add rows: row index → ranges.
pub(super) fn emphasis_for(rows: &[Row]) -> std::collections::HashMap<usize, Vec<(usize, usize)>> {
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

pub(super) fn draw_diff(f: &mut Frame, app: &App, area: Rect) {
    let rows = app.rows();
    let title = app
        .files
        .get(app.diff_file)
        .map(|_| format!("diff: {}", app.current_diff_path().unwrap_or_default()))
        .unwrap_or_else(|| "diff".to_string());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style(app.focus == Focus::Diff));

    if rows.is_empty() {
        let msg = Paragraph::new(Span::styled(
            "clean — no changes",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        f.render_widget(msg, area);
        return;
    }

    let commented = commented_lines(app);
    let emphasis = emphasis_for(&rows);
    let height = area.height.saturating_sub(2) as usize;
    let top = app
        .diff_scroll
        .min(rows.len().saturating_sub(height.max(1)));
    let path = app.current_diff_path().unwrap_or_default();
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
                let side = if l.kind == LineKind::Del {
                    Side::Old
                } else {
                    Side::New
                };
                let mark = if commented.contains(&(side, no)) {
                    "●"
                } else {
                    " "
                };

                let mut text_spans = crate::highlight::highlight_line(&path, &l.text);
                if let Some(ranges) = emphasis.get(&i) {
                    let em = if l.kind == LineKind::Add {
                        ADD_EM_BG
                    } else {
                        DEL_EM_BG
                    };
                    text_spans = apply_bg_ranges(&text_spans, ranges, em);
                }
                let mut content = vec![Span::styled(
                    sign.to_string(),
                    Style::default().fg(sign_color),
                )];
                content.extend(text_spans);
                let mut spans = vec![Span::styled(
                    format!("{mark}{no:>5} "),
                    Style::default().fg(Color::DarkGray),
                )];
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
                    Span::styled(
                        "  (Enter to save, Esc to cancel)",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn commented_lines(app: &App) -> Vec<(Side, u32)> {
    let path = app.comment_path().unwrap_or_default();
    app.store
        .comments(&app.agent_key())
        .iter()
        .filter(|c| c.path == path)
        .map(|c| (c.side.clone(), c.line_no))
        .collect()
}
