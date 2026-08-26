//! Application state: navigation, comments, submit flow. IO-free.

use std::path::Path;

use crate::diff::{DiffLine, FileDiff, LineKind};
use crate::herdr::Agent;
use crate::review::{compose_prompt, Comment, ReviewStore, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

/// What the right-hand pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPane {
    Diff,
    File,
}

/// A renderable row of the diff view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    HunkHeader(String),
    Line(DiffLine),
}

/// Effect the caller (main loop) must perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Refresh,
    /// Send `text` to the agent in `pane_id` and clear its comments on success.
    Submit {
        pane_id: String,
        text: String,
    },
    /// Load file content for the viewer.
    OpenFile {
        path: String,
    },
    ToggleBase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Comment input; buffer holds the typed text.
    Input { buffer: String },
    /// y/n confirmation before prompting a non-idle agent.
    ConfirmSubmit,
}

/// Screen regions computed by the ui layout, used for mouse hit-testing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Regions {
    pub agents: ratatui::layout::Rect,
    pub files: ratatui::layout::Rect,
    pub right: ratatui::layout::Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    LeftClick,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

/// One git repo a window shows changes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub root: std::path::PathBuf,
    pub label: String,
}

pub struct App {
    /// The one agent this window is pinned to; it never switches.
    pub agent: Option<Agent>,
    /// Pane id this window was pinned to at startup.
    pub pinned_pane: Option<String>,
    pub files: Vec<FileDiff>,
    /// Repos the files belong to; empty in plain single-repo mode.
    pub repos: Vec<RepoRef>,
    /// Repo index (into `repos`) per entry of `files`.
    pub file_repo: Vec<usize>,
    /// Row of the Files tree the selection is on.
    pub selected_file: usize,
    /// Index into `files` whose diff the right pane shows.
    pub diff_file: usize,
    pub all_files_mode: bool,
    pub all_files: Vec<String>,
    /// Collapsed directories of the all-files tree.
    pub collapsed: std::collections::BTreeSet<String>,
    /// First visible row of the Files pane.
    pub files_scroll: usize,
    /// Inner height of the Files pane, reported by the ui each frame.
    pub files_viewport: usize,
    /// First visible row of the diff view.
    pub diff_scroll: usize,
    /// Inner height of the right pane, reported by the ui each frame.
    pub diff_viewport: usize,
    pub cursor: usize,
    /// Horizontal scroll of the diff / file view, in characters.
    pub hscroll: u16,
    /// Vertical scroll of the file viewer.
    pub fv_scroll: u16,
    /// Content (new side) of the current diff file; empty → hunk-only view.
    pub file_lines: Vec<String>,
    /// Path `file_lines` belongs to.
    content_path: Option<String>,
    /// Agent `file_lines` was read for (paths can repeat across repos).
    content_agent: String,
    pub focus: Focus,
    pub right: RightPane,
    pub modal: Option<Modal>,
    pub status: String,
    pub standalone: bool,
    pub store: ReviewStore,
}

impl App {
    pub fn new(state_dir: &Path, standalone: bool) -> Self {
        Self {
            agent: None,
            pinned_pane: None,
            files: Vec::new(),
            repos: Vec::new(),
            file_repo: Vec::new(),
            selected_file: 0,
            diff_file: 0,
            all_files_mode: false,
            all_files: Vec::new(),
            collapsed: std::collections::BTreeSet::new(),
            files_scroll: 0,
            files_viewport: 0,
            diff_scroll: 0,
            diff_viewport: 0,
            cursor: 0,
            focus: Focus::Diff,
            right: RightPane::Diff,
            modal: None,
            status: String::new(),
            standalone,
            hscroll: 0,
            fv_scroll: 0,
            file_lines: Vec::new(),
            content_path: None,
            content_agent: String::new(),
            store: ReviewStore::load(state_dir),
        }
    }

    pub fn agent(&self) -> Option<&Agent> {
        self.agent.as_ref()
    }

    /// Pin this window to one agent pane, permanently.
    pub fn pin(&mut self, pane_id: String) {
        self.pinned_pane = Some(pane_id);
    }

    pub fn agent_key(&self) -> String {
        self.agent().map(Agent::key).unwrap_or_else(|| "standalone".to_string())
    }

    /// Update the pinned agent from a fresh agent list. When its pane is
    /// gone, the last known info is kept with status "gone".
    pub fn set_agents(&mut self, agents: Vec<Agent>) {
        let Some(pinned) = self.pinned_pane.clone() else { return };
        match agents.into_iter().find(|a| a.pane_id == pinned) {
            Some(a) => self.agent = Some(a),
            None => {
                if let Some(a) = &mut self.agent {
                    a.agent_status = "gone".to_string();
                }
            }
        }
    }

    /// Path of a file as shown in the Files tree: repo-label-prefixed when
    /// this window spans several repos.
    pub fn display_path(&self, idx: usize) -> Option<String> {
        let f = self.files.get(idx)?;
        if self.repos.len() > 1 {
            let repo = self.repos.get(*self.file_repo.get(idx)?)?;
            Some(format!("{}/{}", repo.label, f.new_path))
        } else {
            Some(f.new_path.clone())
        }
    }

    /// Index of the file whose display path matches.
    pub fn file_index_by_display(&self, display: &str) -> Option<usize> {
        (0..self.files.len()).find(|&i| self.display_path(i).as_deref() == Some(display))
    }

    /// Where the current diff file lives: (repo root if known, repo-relative path).
    pub fn current_file_location(&self) -> Option<(Option<&std::path::Path>, &str)> {
        let f = self.files.get(self.diff_file)?;
        let root = self
            .file_repo
            .get(self.diff_file)
            .and_then(|&r| self.repos.get(r))
            .map(|r| r.root.as_path());
        Some((root, f.new_path.as_str()))
    }

    /// Replace the diff with per-repo groups (the cwd repo plus any repos the
    /// agent touched), keeping the shown file and tree selection when possible.
    pub fn set_multi_diff(&mut self, groups: Vec<(RepoRef, Vec<FileDiff>)>) {
        let prev_diff = self.display_path(self.diff_file);
        let prev_sel = if self.all_files_mode {
            None
        } else {
            self.files_tree_rows().get(self.selected_file).map(|r| r.path.clone())
        };
        self.files.clear();
        self.repos.clear();
        self.file_repo.clear();
        for (i, (repo, files)) in groups.into_iter().enumerate() {
            self.repos.push(repo);
            for f in files {
                self.files.push(f);
                self.file_repo.push(i);
            }
        }
        self.diff_file = prev_diff
            .and_then(|p| self.file_index_by_display(&p))
            .unwrap_or(0);
        if let Some(p) = prev_sel {
            self.selected_file = self
                .files_tree_rows()
                .iter()
                .position(|r| r.path == p)
                .unwrap_or(0);
        }
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
        self.diff_scroll = self.diff_scroll.min(self.rows().len().saturating_sub(1));
    }

    /// Replace the diff, keeping the shown diff file and the tree selection
    /// on the same paths when possible.
    pub fn set_diff(&mut self, files: Vec<FileDiff>) {
        let diff_path = self.files.get(self.diff_file).map(|f| f.new_path.clone());
        let selected_path = if self.all_files_mode {
            None // tree of all files is independent of the diff
        } else {
            self.files_tree_rows().get(self.selected_file).map(|r| r.path.clone())
        };
        self.files = files;
        self.repos.clear();
        self.file_repo.clear();
        self.diff_file = diff_path
            .and_then(|p| self.files.iter().position(|f| f.new_path == p))
            .unwrap_or(0);
        if let Some(p) = selected_path {
            self.selected_file = self
                .files_tree_rows()
                .iter()
                .position(|r| r.path == p)
                .unwrap_or(0);
        }
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
        self.diff_scroll = self.diff_scroll.min(self.rows().len().saturating_sub(1));
    }

    /// Display path of the diff file the right pane currently shows.
    pub fn current_diff_path(&self) -> Option<String> {
        self.display_path(self.diff_file)
    }

    /// Provide the full content of the current diff file. On a new file this
    /// jumps the view to the first change; a reload of the same file (e.g.
    /// background refresh) keeps the reading position.
    pub fn set_diff_content(&mut self, path: &str, lines: Vec<String>) {
        let agent = self.agent_key();
        let new_file = self.content_path.as_deref() != Some(path) || self.content_agent != agent;
        self.content_path = Some(path.to_string());
        self.content_agent = agent;
        self.file_lines = lines;
        let len = self.rows().len();
        if new_file {
            let first_change = self
                .rows()
                .iter()
                .position(|r| matches!(r, Row::Line(l) if l.kind != LineKind::Context))
                .unwrap_or(0);
            self.cursor = first_change;
            self.diff_scroll = first_change.saturating_sub(3);
        } else {
            self.cursor = self.cursor.min(len.saturating_sub(1));
            self.diff_scroll = self.diff_scroll.min(len.saturating_sub(1));
        }
    }

    /// Flattened rows of the current diff file: the whole file with inline
    /// deletions when its content is loaded, hunks only otherwise.
    pub fn rows(&self) -> Vec<Row> {
        let Some(file) = self.files.get(self.diff_file) else {
            return Vec::new();
        };
        if !self.file_lines.is_empty()
            && self.content_path == self.display_path(self.diff_file)
            && self.content_agent == self.agent_key()
        {
            let added: std::collections::BTreeSet<u32> = file
                .hunks
                .iter()
                .flat_map(|h| h.lines.iter())
                .filter(|l| l.kind == LineKind::Add)
                .filter_map(|l| l.new_no)
                .collect();
            let mut dels = crate::diff::deleted_lines_by_anchor(file);
            let mut rows = Vec::with_capacity(self.file_lines.len());
            for (i, text) in self.file_lines.iter().enumerate() {
                let no = i as u32 + 1;
                if let Some(pending) = dels.remove(&no) {
                    rows.extend(pending.into_iter().map(Row::Line));
                }
                let kind = if added.contains(&no) { LineKind::Add } else { LineKind::Context };
                rows.push(Row::Line(DiffLine {
                    kind,
                    old_no: None,
                    new_no: Some(no),
                    text: text.clone(),
                }));
            }
            for (_, pending) in dels {
                rows.extend(pending.into_iter().map(Row::Line));
            }
            return rows;
        }
        let mut rows = Vec::new();
        for hunk in &file.hunks {
            rows.push(Row::HunkHeader(hunk.header.clone()));
            for line in &hunk.lines {
                rows.push(Row::Line(line.clone()));
            }
        }
        rows
    }

    /// Visible rows of the all-files tree.
    pub fn tree_rows(&self) -> Vec<crate::tree::TreeRow> {
        crate::tree::visible_rows(&self.all_files, &self.collapsed)
    }

    /// Visible rows of the Files pane: all files or just the changed ones.
    pub fn files_tree_rows(&self) -> Vec<crate::tree::TreeRow> {
        if self.all_files_mode {
            self.tree_rows()
        } else {
            let paths: Vec<String> =
                (0..self.files.len()).filter_map(|i| self.display_path(i)).collect();
            crate::tree::visible_rows(&paths, &self.collapsed)
        }
    }

    /// Enter on a tree row: toggle a dir or open the file in the viewer.
    fn activate_tree_row(&mut self, idx: usize) -> Action {
        let rows = self.files_tree_rows();
        let Some(row) = rows.get(idx) else { return Action::None };
        if row.is_dir {
            self.toggle_dir(&row.path.clone());
            Action::None
        } else {
            self.right = RightPane::File;
            self.fv_scroll = 0;
            self.hscroll = 0;
            Action::OpenFile { path: row.path.clone() }
        }
    }

    /// Click on a tree row: toggle a dir; a file shows its diff (or opens the
    /// viewer in all-files mode, where most files have no diff).
    fn click_tree_row(&mut self, idx: usize) -> Action {
        let rows = self.files_tree_rows();
        let Some(row) = rows.get(idx) else { return Action::None };
        if row.is_dir {
            self.toggle_dir(&row.path.clone());
            return Action::None;
        }
        if self.all_files_mode {
            return self.activate_tree_row(idx);
        }
        if let Some(i) = self.file_index_by_display(&row.path) {
            self.diff_file = i;
            self.right = RightPane::Diff;
            self.cursor = 0;
            self.hscroll = 0;
            self.diff_scroll = 0;
        }
        Action::None
    }

    fn toggle_dir(&mut self, path: &str) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.selected_file = self.selected_file.min(self.files_tree_rows().len().saturating_sub(1));
    }

    /// After the tree selection moved, point the diff at the selected file.
    fn sync_diff_file(&mut self) {
        if self.all_files_mode {
            return;
        }
        if let Some(row) = self.files_tree_rows().get(self.selected_file) {
            if !row.is_dir {
                if let Some(i) = self.file_index_by_display(&row.path) {
                    if i != self.diff_file {
                        self.diff_file = i;
                        self.cursor = 0;
                        self.diff_scroll = 0;
                    }
                }
            }
        }
    }

    /// Path used in comments for the current diff file: absolute when the
    /// repo root is known, repo-relative otherwise.
    pub fn comment_path(&self) -> Option<String> {
        match self.current_file_location()? {
            (Some(root), rel) => Some(root.join(rel).to_string_lossy().into_owned()),
            (None, rel) => Some(rel.to_string()),
        }
    }

    /// Comment anchor for the current cursor row, if it is a diff line.
    pub fn anchor_at_cursor(&self) -> Option<(String, Side, u32, Vec<String>)> {
        let rows = self.rows();
        let Row::Line(line) = rows.get(self.cursor)? else {
            return None;
        };
        let path = self.comment_path()?;
        let (side, no) = match line.kind {
            LineKind::Del => (Side::Old, line.old_no?),
            _ => (Side::New, line.new_no?),
        };
        let from = self.cursor.saturating_sub(2);
        let to = (self.cursor + 3).min(rows.len());
        let quote = rows[from..to]
            .iter()
            .filter_map(|r| match r {
                Row::Line(l) => Some(format!("{}{}", prefix(&l.kind), l.text)),
                Row::HunkHeader(_) => None,
            })
            .collect();
        Some((path, side, no, quote))
    }

    /// Mouse input mapped through the current screen regions.
    pub fn handle_mouse(&mut self, m: Mouse, x: u16, y: u16, regions: &Regions) -> Action {
        if self.modal.is_some() {
            return Action::None;
        }
        let inside = |r: &ratatui::layout::Rect| {
            x > r.x && x < r.x + r.width.saturating_sub(1) && y > r.y && y < r.y + r.height.saturating_sub(1)
        };
        if inside(&regions.agents) {
            // pinned-agent header: not interactive
            Action::None
        } else if inside(&regions.files) {
            let idx = self.files_scroll + (y - regions.files.y - 1) as usize;
            let len = self.files_tree_rows().len();
            match m {
                Mouse::LeftClick => {
                    if idx >= len {
                        return Action::None;
                    }
                    self.selected_file = idx;
                    self.focus = Focus::Files;
                    self.click_tree_row(idx)
                }
                Mouse::WheelDown | Mouse::WheelUp => {
                    let viewport = if self.files_viewport > 0 {
                        self.files_viewport
                    } else {
                        regions.files.height.saturating_sub(2) as usize
                    };
                    let max = len.saturating_sub(viewport);
                    self.files_scroll = if m == Mouse::WheelDown {
                        (self.files_scroll + 3).min(max)
                    } else {
                        self.files_scroll.saturating_sub(3)
                    };
                    Action::None
                }
                Mouse::WheelLeft | Mouse::WheelRight => Action::None,
            }
        } else if inside(&regions.right) {
            if matches!(m, Mouse::WheelLeft | Mouse::WheelRight) {
                self.hscroll = if m == Mouse::WheelRight {
                    self.hscroll.saturating_add(8)
                } else {
                    self.hscroll.saturating_sub(8)
                };
                return Action::None;
            }
            match self.right {
                RightPane::File => {
                    match m {
                        Mouse::WheelDown => self.fv_scroll = self.fv_scroll.saturating_add(3),
                        Mouse::WheelUp => self.fv_scroll = self.fv_scroll.saturating_sub(3),
                        _ => self.focus = Focus::Diff,
                    }
                    Action::None
                }
                RightPane::Diff => {
                    let len = self.rows().len();
                    if len == 0 {
                        return Action::None;
                    }
                    match m {
                        Mouse::WheelDown | Mouse::WheelUp => {
                            let viewport = if self.diff_viewport > 0 {
                                self.diff_viewport
                            } else {
                                regions.right.height.saturating_sub(2) as usize
                            };
                            let max = len.saturating_sub(viewport);
                            self.diff_scroll = if m == Mouse::WheelDown {
                                (self.diff_scroll + 3).min(max)
                            } else {
                                self.diff_scroll.saturating_sub(3)
                            };
                        }
                        _ => {
                            self.focus = Focus::Diff;
                            let row = self.diff_scroll + (y - regions.right.y - 1) as usize;
                            self.cursor = row.min(len - 1);
                        }
                    }
                    Action::None
                }
            }
        } else {
            Action::None
        }
    }

    pub fn handle_key(&mut self, key: char) -> Action {
        if let Some(modal) = self.modal.take() {
            return self.handle_modal_key(modal, key);
        }
        match key {
            'q' => Action::Quit,
            'r' => Action::Refresh,
            'b' => Action::ToggleBase,
            'a' => {
                self.all_files_mode = !self.all_files_mode;
                self.selected_file = 0;
                self.files_scroll = 0;
                Action::Refresh
            }
            '\t' => {
                self.focus = match self.focus {
                    Focus::Files => Focus::Diff,
                    _ => Focus::Files,
                };
                Action::None
            }
            'j' | 'k' if self.right == RightPane::File && self.focus != Focus::Files => {
                self.fv_scroll = if key == 'j' {
                    self.fv_scroll.saturating_add(1)
                } else {
                    self.fv_scroll.saturating_sub(1)
                };
                Action::None
            }
            'j' | 'k' => {
                self.navigate(key == 'j');
                Action::None
            }
            'h' | 'l' => {
                self.hscroll = if key == 'l' {
                    self.hscroll.saturating_add(8)
                } else {
                    self.hscroll.saturating_sub(8)
                };
                Action::None
            }
            'd' => {
                self.right = RightPane::Diff;
                Action::None
            }
            '\n' if self.focus == Focus::Files => self.activate_tree_row(self.selected_file),
            'c' if self.focus == Focus::Diff && !self.standalone => {
                if self.anchor_at_cursor().is_some() {
                    self.modal = Some(Modal::Input { buffer: String::new() });
                }
                Action::None
            }
            'x' if self.focus == Focus::Diff && !self.standalone => {
                let key = self.agent_key();
                let count = self.store.comments(&key).len();
                if count > 0 {
                    self.store.remove(&key, count - 1);
                }
                Action::None
            }
            'S' if !self.standalone => self.submit(false),
            _ => Action::None,
        }
    }

    fn handle_modal_key(&mut self, modal: Modal, key: char) -> Action {
        match modal {
            Modal::Input { mut buffer } => match key {
                '\n' => {
                    if !buffer.trim().is_empty() {
                        if let Some((path, side, line_no, quote)) = self.anchor_at_cursor() {
                            let agent_key = self.agent_key();
                            self.store.add(
                                &agent_key,
                                Comment { path, side, line_no, quote, text: buffer.trim().to_string() },
                            );
                        }
                    }
                    Action::None
                }
                '\u{1b}' => Action::None,
                '\u{8}' | '\u{7f}' => {
                    buffer.pop();
                    self.modal = Some(Modal::Input { buffer });
                    Action::None
                }
                ch => {
                    buffer.push(ch);
                    self.modal = Some(Modal::Input { buffer });
                    Action::None
                }
            },
            Modal::ConfirmSubmit => {
                if key == 'y' {
                    self.submit(true)
                } else {
                    Action::None
                }
            }
        }
    }

    fn submit(&mut self, confirmed: bool) -> Action {
        let key = self.agent_key();
        if self.store.comments(&key).is_empty() {
            return Action::None;
        }
        let Some(agent) = self.agent() else {
            return Action::None;
        };
        if !confirmed && agent.agent_status != "idle" {
            self.modal = Some(Modal::ConfirmSubmit);
            return Action::None;
        }
        Action::Submit {
            pane_id: agent.pane_id.clone(),
            text: compose_prompt(self.store.comments(&key)),
        }
    }

    fn navigate(&mut self, down: bool) {
        let step = |v: usize, len: usize| -> usize {
            if down {
                (v + 1).min(len.saturating_sub(1))
            } else {
                v.saturating_sub(1)
            }
        };
        match self.focus {
            Focus::Files => {
                let len = self.files_tree_rows().len();
                self.selected_file = step(self.selected_file, len);
                self.hscroll = 0;
                self.sync_diff_file();
                // keep the selection visible
                if self.files_viewport > 0 {
                    if self.selected_file < self.files_scroll {
                        self.files_scroll = self.selected_file;
                    } else if self.selected_file >= self.files_scroll + self.files_viewport {
                        self.files_scroll = self.selected_file + 1 - self.files_viewport;
                    }
                }
            }
            Focus::Diff => {
                self.cursor = step(self.cursor, self.rows().len());
                // keep the cursor visible
                if self.diff_viewport > 0 {
                    if self.cursor < self.diff_scroll {
                        self.diff_scroll = self.cursor;
                    } else if self.cursor >= self.diff_scroll + self.diff_viewport {
                        self.diff_scroll = self.cursor + 1 - self.diff_viewport;
                    }
                }
            }
        }
    }
}

fn prefix(kind: &LineKind) -> &'static str {
    match kind {
        LineKind::Context => " ",
        LineKind::Add => "+",
        LineKind::Del => "-",
    }
}

#[cfg(test)]
mod tests {
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
        // click maps through the scroll offset
        a.diff_scroll = 1;
        a.handle_mouse(Mouse::LeftClick, 30, 2, &regions());
        assert_eq!(a.cursor, 2);
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
        let mut a = app();
        a.right = RightPane::File;
        a.focus = Focus::Diff;
        let before = a.cursor;
        a.handle_key('j');
        assert_eq!(a.fv_scroll, 1);
        assert_eq!(a.cursor, before, "diff cursor untouched while viewing file");
        a.handle_key('k');
        assert_eq!(a.fv_scroll, 0);
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
}
