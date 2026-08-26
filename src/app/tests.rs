//! App tests.

    use super::*;
    use crate::diff::parse;

    fn agent(pane: &str, status: &str, cwd: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "pane_id": pane, "agent": "claude", "agent_status": status, "cwd": cwd
        }))
        .unwrap()
    }

    const DIFF: &str = "\
diff --git a/user.py b/user.py
index 1111111..2222222 100644
--- a/user.py
+++ b/user.py
@@ -41,3 +41,3 @@
 def get_user(id):
-    return None
+    return user
diff --git a/config.py b/config.py
index 1111111..2222222 100644
--- a/config.py
+++ b/config.py
@@ -7,1 +7,1 @@
-TIMEOUT = 5
+TIMEOUT = 30
";

    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::new(dir.path(), false);
        a.pin("w1:p1".to_string());
        a.set_agents(vec![
            agent("w1:p1", "idle", "/repo/api"),
            agent("w1:p2", "working", "/repo/web"),
        ]);
        a.set_diff(parse(DIFF));
        a
    }

    #[test]
    fn set_agents_tracks_only_the_pinned_agent() {
        let mut a = app();
        assert_eq!(a.agent().unwrap().pane_id, "w1:p1");
        // updates flow in for the pinned agent, others are ignored
        a.set_agents(vec![agent("w1:p2", "idle", "/repo/web"), agent("w1:p1", "blocked", "/repo/api")]);
        assert_eq!(a.agent().unwrap().pane_id, "w1:p1");
        assert_eq!(a.agent().unwrap().agent_status, "blocked");
        // pinned pane vanished: keep last info, mark gone
        a.set_agents(vec![agent("w9:p9", "idle", "/x")]);
        assert_eq!(a.agent().unwrap().pane_id, "w1:p1");
        assert_eq!(a.agent().unwrap().agent_status, "gone");
    }

    #[test]
    fn set_diff_preserves_file_selection_by_path() {
        let mut a = app();
        a.selected_file = 1; // tree rows sorted: [config.py, user.py] → user.py
        let mut reordered = parse(DIFF);
        reordered.reverse();
        a.set_diff(reordered);
        assert_eq!(a.files_tree_rows()[a.selected_file].path, "user.py", "selection stays on its row");
    }

    fn content_44() -> Vec<String> {
        // 44-line file whose line 41 is context, 42 is the added line
        (1..=44)
            .map(|i| match i {
                41 => "def get_user(id):".to_string(),
                42 => "    return user".to_string(),
                n => format!("line {n}"),
            })
            .collect()
    }

    #[test]
    fn diff_shows_whole_file_when_content_is_set() {
        let mut a = app();
        a.set_diff_content("user.py", content_44());
        let rows = a.rows();
        // 44 content lines + 1 inline deletion, no hunk headers
        assert_eq!(rows.len(), 45);
        assert!(rows.iter().all(|r| matches!(r, Row::Line(_))));
        assert!(matches!(&rows[0], Row::Line(l) if l.kind == LineKind::Context && l.text == "line 1"));
        // deletion sits right before the line that replaced it
        assert!(matches!(&rows[41], Row::Line(l) if l.kind == LineKind::Del && l.text == "    return None"));
        assert!(matches!(&rows[42], Row::Line(l) if l.kind == LineKind::Add && l.new_no == Some(42)));
        assert!(matches!(&rows[44], Row::Line(l) if l.kind == LineKind::Context && l.new_no == Some(44)));
    }

    #[test]
    fn opening_a_file_jumps_to_first_change() {
        let mut a = app();
        a.set_diff_content("user.py", content_44());
        assert_eq!(a.cursor, 41, "cursor on the first changed row");
        assert_eq!(a.diff_scroll, 38, "scrolled a little above it");
        // reloading the same file (background refresh) keeps the position
        a.cursor = 5;
        a.diff_scroll = 2;
        a.set_diff_content("user.py", content_44());
        assert_eq!((a.cursor, a.diff_scroll), (5, 2));
    }

    #[test]
    fn rows_fall_back_to_hunks_without_content() {
        let a = app();
        assert!(matches!(a.rows()[0], Row::HunkHeader(_)));
    }

    #[test]
    fn rows_flatten_headers_and_lines() {
        let a = app();
        let rows = a.rows();
        assert!(matches!(rows[0], Row::HunkHeader(_)));
        assert_eq!(rows.len(), 4); // header + 3 lines
    }

    #[test]
    fn anchor_uses_new_side_for_add_and_context() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.cursor = 3; // "+    return user"
        let (path, side, line, quote) = a.anchor_at_cursor().unwrap();
        assert_eq!(path, "user.py");
        assert_eq!(side, Side::New);
        assert_eq!(line, 42);
        assert!(quote.iter().any(|q| q == "+    return user"));
        a.cursor = 2; // "-    return None"
        let (_, side, line, _) = a.anchor_at_cursor().unwrap();
        assert_eq!(side, Side::Old);
        assert_eq!(line, 42);
        a.cursor = 0; // hunk header
        assert!(a.anchor_at_cursor().is_none());
    }

    #[test]
    fn comment_flow_add_and_cancel() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.cursor = 3;
        assert_eq!(a.handle_key('c'), Action::None);
        assert!(matches!(a.modal, Some(Modal::Input { .. })));
        for ch in "fix it".chars() {
            a.handle_key(ch);
        }
        a.handle_key('\n');
        assert!(a.modal.is_none());
        assert_eq!(a.store.comments(&a.agent_key()).len(), 1);
        assert_eq!(a.store.comments(&a.agent_key())[0].text, "fix it");
        // esc cancels without adding
        a.handle_key('c');
        a.handle_key('x');
        a.handle_key('\u{1b}');
        assert_eq!(a.store.comments(&a.agent_key()).len(), 1);
    }

    #[test]
    fn empty_comment_is_not_added() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.cursor = 3;
        a.handle_key('c');
        a.handle_key('\n');
        assert!(a.store.comments(&a.agent_key()).is_empty());
    }

    #[test]
    fn submit_idle_agent_returns_prompt_action() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.cursor = 3;
        a.handle_key('c');
        for ch in "note".chars() {
            a.handle_key(ch);
        }
        a.handle_key('\n');
        let action = a.handle_key('S');
        match action {
            Action::Submit { pane_id, text } => {
                assert_eq!(pane_id, "w1:p1");
                assert!(text.contains("user.py:42"));
                assert!(text.contains("note"));
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }

    #[test]
    fn submit_working_agent_requires_confirmation() {
        let mut a = app();
        a.pin("w1:p2".to_string()); // working agent
        a.set_agents(vec![
            agent("w1:p1", "idle", "/repo/api"),
            agent("w1:p2", "working", "/repo/web"),
        ]);
        a.set_diff(parse(DIFF));
        a.focus = Focus::Diff;
        a.cursor = 3;
        a.handle_key('c');
        a.handle_key('n');
        a.handle_key('\n');
        assert_eq!(a.handle_key('S'), Action::None);
        assert_eq!(a.modal, Some(Modal::ConfirmSubmit));
        match a.handle_key('y') {
            Action::Submit { pane_id, .. } => assert_eq!(pane_id, "w1:p2"),
            other => panic!("expected Submit, got {other:?}"),
        }
        assert!(a.modal.is_none());
    }

    #[test]
    fn submit_without_comments_is_noop() {
        let mut a = app();
        assert_eq!(a.handle_key('S'), Action::None);
        assert!(a.modal.is_none());
    }

    #[test]
    fn navigation_stays_in_bounds() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.cursor = 0;
        a.handle_key('j');
        assert_eq!(a.cursor, 1);
        for _ in 0..99 {
            a.handle_key('j');
        }
        assert_eq!(a.cursor, a.rows().len() - 1);
    }

    #[test]
    fn quit_and_refresh_and_base_toggle_actions() {
        let mut a = app();
        assert_eq!(a.handle_key('q'), Action::Quit);
        assert_eq!(a.handle_key('r'), Action::Refresh);
        assert_eq!(a.handle_key('b'), Action::ToggleBase);
    }

    #[test]
    fn enter_on_file_opens_viewer() {
        let mut a = app();
        a.focus = Focus::Files;
        a.selected_file = 1; // tree rows sorted: [config.py, user.py]
        match a.handle_key('\n') {
            Action::OpenFile { path } => assert_eq!(path, "user.py"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
        assert_eq!(a.right, RightPane::File);
        // 'd' returns to diff
        a.handle_key('d');
        assert_eq!(a.right, RightPane::Diff);
    }

    fn regions() -> Regions {
        use ratatui::layout::Rect;
        Regions {
            agents: Rect::new(0, 0, 26, 6),   // inner rows: y 1..=4
            files: Rect::new(0, 6, 26, 10),   // inner rows: y 7..=14
            right: Rect::new(26, 0, 60, 16),  // inner rows: y 1..=14
        }
    }

    #[test]
    fn agent_header_is_not_interactive() {
        let mut a = app();
        let before = a.agent().unwrap().pane_id.clone();
        assert_eq!(a.handle_mouse(Mouse::LeftClick, 3, 2, &regions()), Action::None);
        assert_eq!(a.handle_mouse(Mouse::WheelDown, 3, 2, &regions()), Action::None);
        assert_eq!(a.agent().unwrap().pane_id, before, "pinned agent never changes");
        assert_eq!(a.focus, Focus::Diff, "focus untouched");
    }

    #[test]
    fn click_on_file_selects_and_shows_diff() {
        let mut a = app();
        a.right = RightPane::File;
        let action = a.handle_mouse(Mouse::LeftClick, 3, 8, &regions()); // 2nd file row
        assert_eq!(a.selected_file, 1);
        assert_eq!(a.focus, Focus::Files);
        assert_eq!(a.right, RightPane::Diff);
        assert_eq!(action, Action::None);
    }

    #[test]
    fn click_on_diff_row_moves_cursor() {
        let mut a = app();
        a.focus = Focus::Files;
        // scroll offset 0; y=3 → row 2
        a.handle_mouse(Mouse::LeftClick, 30, 3, &regions());
        assert_eq!(a.focus, Focus::Diff);
        assert_eq!(a.cursor, 2);
        // click below the last row clamps to last
        a.handle_mouse(Mouse::LeftClick, 30, 12, &regions());
        assert_eq!(a.cursor, a.rows().len() - 1);
        // click maps through the scroll offset (small viewport → scroll is real)
        a.diff_viewport = 2;
        a.diff_scroll = 1;
        a.handle_mouse(Mouse::LeftClick, 30, 2, &regions());
        assert_eq!(a.cursor, 2);
    }

    #[test]
    fn click_uses_the_same_clamped_top_as_rendering() {
        let mut a = app(); // 4 diff rows
        a.diff_viewport = 14; // tall pane: whole file fits, draw clamps top to 0
        a.diff_scroll = 10; // stale jump-to-first-change offset
        a.handle_mouse(Mouse::LeftClick, 30, 3, &regions()); // visual row 2
        assert_eq!(a.cursor, 2, "clicked row, not 10 rows below");
        // same for the files pane
        a.files_viewport = 10;
        a.files_scroll = 5; // stale, but the 2-row tree fits entirely
        a.handle_mouse(Mouse::LeftClick, 3, 8, &regions()); // visual row 1
        assert_eq!(a.selected_file, 1);
    }

    #[test]
    fn diff_wheel_scrolls_view_not_cursor() {
        let mut a = app(); // 4 diff rows
        a.diff_viewport = 2;
        a.cursor = 0;
        a.handle_mouse(Mouse::WheelDown, 30, 5, &regions());
        assert_eq!(a.cursor, 0, "cursor untouched");
        assert_eq!(a.diff_scroll, 2, "view scrolled, clamped to len-viewport");
        a.handle_mouse(Mouse::WheelUp, 30, 5, &regions());
        assert_eq!(a.diff_scroll, 0);
        // wheel over file viewer scrolls it
        a.right = RightPane::File;
        a.handle_mouse(Mouse::WheelDown, 30, 5, &regions());
        assert_eq!(a.fv_scroll, 3);
        a.handle_mouse(Mouse::WheelUp, 30, 5, &regions());
        assert_eq!(a.fv_scroll, 0);
    }

    #[test]
    fn diff_cursor_keys_keep_cursor_visible() {
        let mut a = app(); // 4 rows
        a.focus = Focus::Diff;
        a.diff_viewport = 2;
        for _ in 0..3 {
            a.handle_key('j');
        }
        assert_eq!(a.cursor, 3);
        assert_eq!(a.diff_scroll, 2, "view follows cursor down");
        for _ in 0..3 {
            a.handle_key('k');
        }
        assert_eq!(a.diff_scroll, 0, "view follows cursor up");
    }

    #[test]
    fn horizontal_wheel_scrolls_right_pane() {
        let mut a = app();
        a.handle_mouse(Mouse::WheelRight, 30, 5, &regions());
        a.handle_mouse(Mouse::WheelRight, 30, 5, &regions());
        assert_eq!(a.hscroll, 16);
        a.handle_mouse(Mouse::WheelLeft, 30, 5, &regions());
        assert_eq!(a.hscroll, 8);
        // in file view too
        a.right = RightPane::File;
        a.handle_mouse(Mouse::WheelRight, 30, 5, &regions());
        assert_eq!(a.hscroll, 16);
        // outside the right pane: ignored
        a.handle_mouse(Mouse::WheelRight, 3, 2, &regions());
        assert_eq!(a.hscroll, 16);
    }

    #[test]
    fn mouse_ignored_while_typing_comment() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.cursor = 3;
        a.handle_key('c');
        let action = a.handle_mouse(Mouse::LeftClick, 3, 8, &regions());
        assert_eq!(action, Action::None);
        assert_eq!(a.selected_file, 0, "file click ignored while typing");
        assert!(a.modal.is_some());
    }

    #[test]
    fn horizontal_scroll_keys_move_and_clamp() {
        let mut a = app();
        a.focus = Focus::Diff;
        assert_eq!(a.handle_key('h'), Action::None);
        assert_eq!(a.hscroll, 0, "clamped at zero");
        a.handle_key('l');
        a.handle_key('l');
        assert_eq!(a.hscroll, 16, "8 chars per step");
        a.handle_key('h');
        assert_eq!(a.hscroll, 8);
    }

    #[test]
    fn hscroll_resets_on_file_change() {
        let mut a = app();
        a.focus = Focus::Diff;
        a.handle_key('l');
        assert_eq!(a.hscroll, 8);
        a.focus = Focus::Files;
        a.handle_key('j');
        assert_eq!(a.hscroll, 0);
    }

    #[test]
    fn jk_scroll_file_viewer_when_open() {
        let mut a = viewer_app();
        a.diff_viewport = 2;
        let before = a.cursor;
        a.handle_key('j');
        assert_eq!(a.fv_cursor, 1, "cursor moves in the viewer");
        assert_eq!(a.cursor, before, "diff cursor untouched while viewing file");
        a.handle_key('j');
        assert_eq!(a.fv_scroll, 1, "view follows the cursor past the viewport");
        a.handle_key('k');
        a.handle_key('k');
        assert_eq!((a.fv_cursor, a.fv_scroll), (0, 0));
    }

    fn viewer_app() -> App {
        let mut a = app();
        a.focus = Focus::Files;
        a.selected_file = 1; // user.py in the changed tree
        match a.handle_key('\n') {
            Action::OpenFile { path } => assert_eq!(path, "user.py"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
        a.set_view(
            "user.py",
            vec![
                (Some(1), "import os".to_string()),
                (None, "removed = 1".to_string()), // inline deleted row
                (Some(2), "def get_user(id):".to_string()),
                (Some(3), "    return user".to_string()),
            ],
        );
        a.focus = Focus::Diff;
        a
    }

    #[test]
    fn viewer_comment_on_any_line_including_unchanged() {
        let mut a = viewer_app();
        a.fv_cursor = 0; // "import os" — an unchanged line
        assert_eq!(a.handle_key('c'), Action::None);
        assert!(matches!(a.modal, Some(Modal::Input { .. })), "input opens in viewer");
        for ch in "add typing".chars() {
            a.handle_key(ch);
        }
        a.handle_key('\n');
        let comments = a.store.comments(&a.agent_key());
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].path, "user.py");
        assert_eq!(comments[0].line_no, 1);
        assert_eq!(comments[0].text, "add typing");
        assert!(comments[0].quote.iter().any(|q| q.contains("import os")));
    }

    #[test]
    fn viewer_comment_ignored_on_deleted_rows() {
        let mut a = viewer_app();
        a.fv_cursor = 1; // deleted row has no line number
        a.handle_key('c');
        assert!(a.modal.is_none(), "no input on a deleted row");
    }

    #[test]
    fn viewer_click_places_cursor_through_scroll() {
        let mut a = viewer_app();
        a.fv_scroll = 1;
        a.handle_mouse(Mouse::LeftClick, 30, 3, &regions()); // inner row 2 → 1+2=3
        assert_eq!(a.fv_cursor, 3);
    }

    #[test]
    fn viewer_comment_uses_repo_path_in_multi_mode() {
        let mut a = multi_app();
        a.focus = Focus::Files;
        a.selected_file = 2; // api/user.py in the multi tree
        match a.handle_key('\n') {
            Action::OpenFile { path } => assert_eq!(path, "api/user.py"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
        a.set_view("api/user.py", vec![(Some(1), "import os".to_string())]);
        a.focus = Focus::Diff;
        a.fv_cursor = 0;
        a.handle_key('c');
        for ch in "hm".chars() {
            a.handle_key(ch);
        }
        a.handle_key('\n');
        let comments = a.store.comments(&a.agent_key());
        assert_eq!(comments[0].path, "/repo/api/user.py", "absolute path for the agent");
    }

    fn tree_app() -> App {
        let mut a = app();
        a.all_files_mode = true;
        a.all_files = ["src/main.rs", "src/app.rs", "user.py", "README.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        a.focus = Focus::Files;
        a.selected_file = 0;
        a
    }

    #[test]
    fn tree_rows_expose_dirs_and_files() {
        let a = tree_app();
        let rows = a.tree_rows();
        let names: Vec<(&str, bool)> = rows.iter().map(|r| (r.name.as_str(), r.is_dir)).collect();
        assert_eq!(
            names,
            vec![("src", true), ("app.rs", false), ("main.rs", false), ("README.md", false), ("user.py", false)]
        );
    }

    #[test]
    fn enter_on_dir_toggles_collapse() {
        let mut a = tree_app();
        assert_eq!(a.handle_key('\n'), Action::None); // on "src"
        let rows = a.tree_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md", "user.py"]);
        a.handle_key('\n'); // expand back
        assert_eq!(a.tree_rows().len(), 5);
    }

    #[test]
    fn enter_on_tree_file_opens_viewer() {
        let mut a = tree_app();
        a.selected_file = 2; // main.rs
        match a.handle_key('\n') {
            Action::OpenFile { path } => assert_eq!(path, "src/main.rs"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
        assert_eq!(a.right, RightPane::File);
    }

    #[test]
    fn click_on_tree_dir_toggles_and_file_opens() {
        let mut a = tree_app();
        // files region inner rows start at y=7 (see regions()): row 0 = "src"
        let action = a.handle_mouse(Mouse::LeftClick, 3, 7, &regions());
        assert_eq!(action, Action::None);
        assert_eq!(a.tree_rows().len(), 3, "src collapsed by click");
        // row 2 is now "user.py"
        match a.handle_mouse(Mouse::LeftClick, 3, 9, &regions()) {
            Action::OpenFile { path } => assert_eq!(path, "user.py"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
        assert_eq!(a.right, RightPane::File);
    }

    #[test]
    fn files_wheel_scrolls_view_not_selection() {
        let mut a = tree_app(); // 5 tree rows
        a.files_viewport = 3;
        a.handle_mouse(Mouse::WheelDown, 3, 8, &regions());
        assert_eq!(a.selected_file, 0, "selection untouched");
        assert_eq!(a.files_scroll, 2, "view scrolled, clamped to len-viewport");
        a.handle_mouse(Mouse::WheelDown, 3, 8, &regions());
        assert_eq!(a.files_scroll, 2, "clamped at bottom");
        a.handle_mouse(Mouse::WheelUp, 3, 8, &regions());
        assert_eq!(a.files_scroll, 0);
    }

    #[test]
    fn files_click_maps_through_scroll_offset() {
        let mut a = tree_app();
        a.files_viewport = 3;
        a.files_scroll = 2; // visible rows start at "main.rs"
        // y=7 is the first visible row → tree row 2 = src/main.rs
        match a.handle_mouse(Mouse::LeftClick, 3, 7, &regions()) {
            Action::OpenFile { path } => assert_eq!(path, "src/main.rs"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
        assert_eq!(a.selected_file, 2);
    }

    #[test]
    fn keyboard_navigation_keeps_selection_visible() {
        let mut a = tree_app();
        a.files_viewport = 2;
        for _ in 0..3 {
            a.handle_key('j');
        }
        assert_eq!(a.selected_file, 3);
        assert_eq!(a.files_scroll, 2, "view follows selection down");
        for _ in 0..3 {
            a.handle_key('k');
        }
        assert_eq!(a.selected_file, 0);
        assert_eq!(a.files_scroll, 0, "view follows selection up");
    }

    const NESTED_DIFF: &str = "\
diff --git a/src/app.rs b/src/app.rs
index 1111111..2222222 100644
--- a/src/app.rs
+++ b/src/app.rs
@@ -1,1 +1,1 @@
-old
+new
diff --git a/README.md b/README.md
index 1111111..2222222 100644
--- a/README.md
+++ b/README.md
@@ -1,1 +1,1 @@
-a
+b
";

    #[test]
    fn changed_files_render_as_tree() {
        let mut a = app();
        a.set_diff(parse(NESTED_DIFF));
        let rows = a.files_tree_rows();
        let names: Vec<(&str, bool)> = rows.iter().map(|r| (r.name.as_str(), r.is_dir)).collect();
        assert_eq!(names, vec![("src", true), ("app.rs", false), ("README.md", false)]);
    }

    #[test]
    fn changed_tree_dir_toggles_and_file_shows_diff() {
        let mut a = app();
        a.set_diff(parse(NESTED_DIFF));
        a.focus = Focus::Files;
        a.selected_file = 0; // "src"
        assert_eq!(a.handle_key('\n'), Action::None);
        assert_eq!(a.files_tree_rows().len(), 2, "src collapsed");
        a.handle_key('\n'); // expand back
        // navigate onto src/app.rs → its diff becomes current
        a.selected_file = 0;
        a.handle_key('j'); // onto app.rs
        assert!(matches!(&a.rows()[1], Row::Line(l) if l.text == "old"),
            "diff follows selected file");
        // Enter on the file opens the viewer
        match a.handle_key('\n') {
            Action::OpenFile { path } => assert_eq!(path, "src/app.rs"),
            other => panic!("expected OpenFile, got {other:?}"),
        }
    }

    #[test]
    fn changed_tree_click_file_shows_its_diff() {
        let mut a = app();
        a.set_diff(parse(NESTED_DIFF));
        // rows: src(y7), app.rs(y8), README.md(y9)
        a.handle_mouse(Mouse::LeftClick, 3, 9, &regions());
        assert_eq!(a.right, RightPane::Diff);
        assert!(matches!(&a.rows()[1], Row::Line(l) if l.text == "a"),
            "README diff shown after click");
    }

    #[test]
    fn set_diff_preserves_current_diff_file_by_path() {
        let mut a = app();
        a.set_diff(parse(NESTED_DIFF));
        a.handle_mouse(Mouse::LeftClick, 3, 9, &regions()); // README.md
        let mut reordered = parse(NESTED_DIFF);
        reordered.reverse();
        a.set_diff(reordered);
        assert!(matches!(&a.rows()[1], Row::Line(l) if l.text == "a"),
            "diff file preserved across refresh");
    }

    fn multi_app() -> App {
        let mut a = app();
        a.set_multi_diff(vec![
            (
                RepoRef { root: "/repo/api".into(), label: "api".into() },
                parse(DIFF), // user.py, config.py
            ),
            (
                RepoRef { root: "/repo/web".into(), label: "web".into() },
                parse(NESTED_DIFF), // src/app.rs, README.md
            ),
        ]);
        a
    }

    #[test]
    fn multi_repo_tree_gets_repo_roots_as_top_dirs() {
        let a = multi_app();
        let rows = a.files_tree_rows();
        let names: Vec<(usize, &str, bool)> =
            rows.iter().map(|r| (r.depth, r.name.as_str(), r.is_dir)).collect();
        assert_eq!(
            names,
            vec![
                (0, "api", true),
                (1, "config.py", false),
                (1, "user.py", false),
                (0, "web", true),
                (1, "src", true),
                (2, "app.rs", false),
                (1, "README.md", false),
            ]
        );
    }

    #[test]
    fn single_repo_group_keeps_plain_tree() {
        let mut a = app();
        a.set_multi_diff(vec![(
            RepoRef { root: "/repo/api".into(), label: "api".into() },
            parse(DIFF),
        )]);
        let rows = a.files_tree_rows();
        assert_eq!(rows[0].name, "config.py", "no repo prefix for a single repo");
        // but the root is still known for file reads
        let (root, rel) = a.current_file_location().unwrap();
        assert_eq!(root.unwrap(), std::path::Path::new("/repo/api"));
        assert_eq!(rel, "user.py");
    }

    #[test]
    fn multi_repo_click_shows_diff_of_other_repo_file() {
        let mut a = multi_app();
        // rows: api(y7), config.py(y8), user.py(y9), web(y10), src(y11), app.rs(y12), README.md(y13)
        a.handle_mouse(Mouse::LeftClick, 3, 12, &regions()); // web/src/app.rs
        assert!(matches!(&a.rows()[1], Row::Line(l) if l.text == "old"), "web repo diff shown");
        let (root, rel) = a.current_file_location().unwrap();
        assert_eq!(root.unwrap(), std::path::Path::new("/repo/web"));
        assert_eq!(rel, "src/app.rs");
    }

    #[test]
    fn multi_repo_comment_anchor_uses_absolute_path() {
        let mut a = multi_app();
        a.focus = Focus::Diff;
        a.cursor = 3; // "+    return user" of api/user.py (hunk fallback rows)
        let (path, _, line, _) = a.anchor_at_cursor().unwrap();
        assert_eq!(path, "/repo/api/user.py");
        assert_eq!(line, 42);
    }

    #[test]
    fn multi_repo_refresh_preserves_current_file_by_display_path() {
        let mut a = multi_app();
        a.handle_mouse(Mouse::LeftClick, 3, 12, &regions()); // web/src/app.rs
        // same groups reversed
        a.set_multi_diff(vec![
            (RepoRef { root: "/repo/web".into(), label: "web".into() }, parse(NESTED_DIFF)),
            (RepoRef { root: "/repo/api".into(), label: "api".into() }, parse(DIFF)),
        ]);
        let (root, rel) = a.current_file_location().unwrap();
        assert_eq!(root.unwrap(), std::path::Path::new("/repo/web"));
        assert_eq!(rel, "src/app.rs");
    }

    #[test]
    fn refresh_keeps_tree_selection() {
        let mut a = tree_app();
        a.selected_file = 3; // README.md in the tree
        a.set_diff(parse(DIFF)); // periodic refresh path
        assert_eq!(a.selected_file, 3, "tree selection survives diff refresh");
    }

    #[test]
    fn tree_navigation_bounded_by_visible_rows() {
        let mut a = tree_app();
        for _ in 0..99 {
            a.handle_key('j');
        }
        assert_eq!(a.selected_file, a.tree_rows().len() - 1);
    }

    #[test]
    fn standalone_disables_submit() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::new(dir.path(), true);
        a.set_diff(parse(DIFF));
        a.focus = Focus::Diff;
        a.cursor = 3;
        assert_eq!(a.handle_key('c'), Action::None);
        assert!(a.modal.is_none(), "comments disabled in standalone mode");
        assert_eq!(a.handle_key('S'), Action::None);
    }
