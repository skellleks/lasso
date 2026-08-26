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
    /// Cursor row of the file viewer.
    pub fv_cursor: usize,
    /// Display path of the file open in the viewer.
    pub viewing: Option<String>,
    /// Viewer rows: (content line number, text); None = inline deleted row.
    pub view_model: Vec<(Option<u32>, String)>,
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
            fv_cursor: 0,
            viewing: None,
            view_model: Vec::new(),
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
        self.agent()
            .map(Agent::key)
            .unwrap_or_else(|| "standalone".to_string())
    }

    /// Update the pinned agent from a fresh agent list. When its pane is
    /// gone, the last known info is kept with status "gone".
    pub fn set_agents(&mut self, agents: Vec<Agent>) {
        let Some(pinned) = self.pinned_pane.clone() else {
            return;
        };
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
            self.files_tree_rows()
                .get(self.selected_file)
                .map(|r| r.path.clone())
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
            self.files_tree_rows()
                .get(self.selected_file)
                .map(|r| r.path.clone())
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
                let kind = if added.contains(&no) {
                    LineKind::Add
                } else {
                    LineKind::Context
                };
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
            let paths: Vec<String> = (0..self.files.len())
                .filter_map(|i| self.display_path(i))
                .collect();
            crate::tree::visible_rows(&paths, &self.collapsed)
        }
    }

    /// Enter on a tree row: toggle a dir or open the file in the viewer.
    fn activate_tree_row(&mut self, idx: usize) -> Action {
        let rows = self.files_tree_rows();
        let Some(row) = rows.get(idx) else {
            return Action::None;
        };
        if row.is_dir {
            self.toggle_dir(&row.path.clone());
            Action::None
        } else {
            self.right = RightPane::File;
            self.focus = Focus::Diff; // move into the opened file
            self.fv_scroll = 0;
            self.fv_cursor = 0;
            self.hscroll = 0;
            Action::OpenFile {
                path: row.path.clone(),
            }
        }
    }

    /// Click on a tree row: toggle a dir; a file shows its diff (or opens the
    /// viewer in all-files mode, where most files have no diff).
    fn click_tree_row(&mut self, idx: usize) -> Action {
        let rows = self.files_tree_rows();
        let Some(row) = rows.get(idx) else {
            return Action::None;
        };
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
        self.selected_file = self
            .selected_file
            .min(self.files_tree_rows().len().saturating_sub(1));
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

    /// Provide the viewer's rows for the file it shows.
    pub fn set_view(&mut self, display: &str, model: Vec<(Option<u32>, String)>) {
        self.viewing = Some(display.to_string());
        self.view_model = model;
        self.fv_cursor = self.fv_cursor.min(self.view_model.len().saturating_sub(1));
    }

    /// Resolve a Files-tree display path to what a comment should carry:
    /// absolute when the repo root is known, as-is otherwise.
    pub fn path_for_display(&self, display: &str) -> String {
        if let Some(i) = self.file_index_by_display(display) {
            if let Some(root) = self.file_repo.get(i).and_then(|&r| self.repos.get(r)) {
                return root
                    .root
                    .join(&self.files[i].new_path)
                    .to_string_lossy()
                    .into_owned();
            }
            return self.files[i].new_path.clone();
        }
        for r in &self.repos {
            if let Some(rest) = display.strip_prefix(&format!("{}/", r.label)) {
                return r.root.join(rest).to_string_lossy().into_owned();
            }
        }
        if let Some(r) = self.repos.first() {
            return r.root.join(display).to_string_lossy().into_owned();
        }
        display.to_string()
    }

    /// Comment path for the file open in the viewer.
    pub fn view_comment_path(&self) -> Option<String> {
        self.viewing.as_deref().map(|d| self.path_for_display(d))
    }

    /// Comment anchor for the viewer cursor; None on deleted rows.
    fn anchor_at_view_cursor(&self) -> Option<(String, Side, u32, Vec<String>)> {
        let (no, _) = self.view_model.get(self.fv_cursor)?;
        let no = (*no)?;
        let path = self.view_comment_path()?;
        let from = self.fv_cursor.saturating_sub(2);
        let to = (self.fv_cursor + 3).min(self.view_model.len());
        let quote = self.view_model[from..to]
            .iter()
            .map(|(n, text)| format!("{}{}", if n.is_some() { " " } else { "-" }, text))
            .collect();
        Some((path, Side::New, no, quote))
    }

    /// Anchor for a new comment wherever the user currently is.
    fn current_anchor(&self) -> Option<(String, Side, u32, Vec<String>)> {
        if self.right == RightPane::File {
            self.anchor_at_view_cursor()
        } else {
            self.anchor_at_cursor()
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
}

fn prefix(kind: &LineKind) -> &'static str {
    match kind {
        LineKind::Context => " ",
        LineKind::Add => "+",
        LineKind::Del => "-",
    }
}

mod input;

#[cfg(test)]
mod tests;
