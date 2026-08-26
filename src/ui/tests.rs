//! UI rendering tests.

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
            FvLine::Content {
                line: Line::from("import os"),
                changed: false,
            },
            FvLine::Deleted {
                text: "removed_line = 0".into(),
            },
            FvLine::Content {
                line: Line::from("changed_line = 1"),
                changed: true,
            },
        ],
    };
    let screen = render(&app, &fv);
    let lines: Vec<&str> = screen.lines().collect();
    let import_row = lines.iter().position(|l| l.contains("import os")).unwrap();
    assert!(lines[import_row].contains("    1 "), "content numbered");
    assert!(
        lines[import_row + 1].contains("- removed_line = 0"),
        "deleted inline"
    );
    assert!(
        lines[import_row + 2].contains("    2 "),
        "numbering skips deleted rows"
    );
    assert!(lines[import_row + 2].contains("changed_line = 1"));
}

#[test]
fn draws_all_files_tree_with_change_badges() {
    let mut app = test_app();
    app.all_files_mode = true;
    app.all_files = ["src/lib.rs", "user.py", "README.md"]
        .iter()
        .map(|s| s.to_string())
        .collect();
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
    let code_row = lines
        .iter()
        .position(|l| l.contains("return user"))
        .unwrap();
    assert!(
        lines[code_row + 1].contains("hi"),
        "input right under the commented line:\n{screen}"
    );
    assert!(
        !screen.contains("Comment (Enter"),
        "no centered popup for input"
    );
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
        let line: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
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
    let spans = vec![
        Span::styled("abc", Style::default().fg(Color::Red)),
        Span::raw("defg"),
    ];
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
    assert!(
        map.get(&1).is_some_and(|r| !r.is_empty()),
        "del row emphasized: {map:?}"
    );
    assert!(
        map.get(&2).is_some_and(|r| !r.is_empty()),
        "add row emphasized"
    );
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
        let line: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        if line.contains("return user") {
            let x = line.find("return user").unwrap() as u16;
            let style = buf[(x, y)].style();
            assert!(
                !style
                    .add_modifier
                    .contains(ratatui::style::Modifier::REVERSED),
                "no reversed video on the cursor line"
            );
            assert_eq!(
                style.bg,
                Some(CUR_ADD_BG),
                "cursor uses a brighter add band"
            );
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
    let line: String = (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
        .collect();
    assert!(line.contains("nav"), "labels present: {line}");
    assert!(line.contains("submit"));
    // the key part is styled differently from its label
    let kx = line.find("j/k").unwrap() as u16;
    let lx = line.find("nav").unwrap() as u16;
    assert_ne!(
        buf[(kx, y)].style().fg,
        buf[(lx, y)].style().fg,
        "key chip differs from label"
    );
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
        let line: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            line.contains("quit"),
            "quit labeled at width {width}: {line}"
        );
        assert!(
            line.contains("submit"),
            "labels never dropped at width {width}: {line}"
        );
        if width == 90 {
            assert!(
                line.contains("refresh"),
                "medium width keeps secondary hints: {line}"
            );
        }
        let end = line.rfind("quit").unwrap() + 4;
        assert!(
            end as u16 >= width.saturating_sub(3),
            "hints fill the width at {width}: end={end}, line: {line}"
        );
    }
}

#[test]
fn viewer_cursor_band_marker_and_inline_input() {
    let mut app = test_app();
    app.focus = Focus::Files;
    app.selected_file = 0; // user.py — the only changed file in this fixture
    assert!(matches!(
        app.handle_key('\n'),
        crate::app::Action::OpenFile { .. }
    ));
    app.set_view(
        "user.py",
        vec![
            (Some(1), "import os".to_string()),
            (Some(2), "def get_user(id):".to_string()),
        ],
    );
    app.focus = Focus::Diff;
    let fv = FileView {
        path: "user.py".into(),
        lines: vec![
            FvLine::Content {
                line: Line::from("import os"),
                changed: false,
            },
            FvLine::Content {
                line: Line::from("def get_user(id):"),
                changed: false,
            },
        ],
    };
    // comment on line 1, then open input on line 2
    app.fv_cursor = 0;
    app.handle_key('c');
    for ch in "note".chars() {
        app.handle_key(ch);
    }
    app.handle_key('\n');
    app.handle_key('j');
    app.handle_key('c');
    app.handle_key('h');
    app.handle_key('i');
    let screen = render(&app, &fv);
    let lines: Vec<&str> = screen.lines().collect();
    let import_row = lines.iter().position(|l| l.contains("import os")).unwrap();
    assert!(
        lines[import_row].contains('●'),
        "comment marker on line 1: {}",
        lines[import_row]
    );
    let def_row = lines
        .iter()
        .position(|l| l.contains("def get_user"))
        .unwrap();
    assert!(
        lines[def_row + 1].contains("hi"),
        "inline input under viewer cursor"
    );
}

#[test]
fn empty_app_renders_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(dir.path(), true);
    let screen = render(&app, &FileView::default());
    assert!(
        screen.contains("clean") || screen.contains("no changes"),
        "placeholder shown"
    );
}
