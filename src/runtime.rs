//! Everything around the TUI loop: pane opener, refresh pipeline,
//! transcript discovery, file loading and the socket listener.

use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::KeyCode;

use crate::app::{self, Action, App, Mouse};
use crate::gitio::{self, DiffBase};
use crate::ui::{self, FileView};
use crate::{diff, herdr, highlight, transcript};

pub(crate) enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Mouse(Mouse, u16, u16),
    AgentsChanged,
    Tick,
}

/// Action entrypoint: open a review pane next to the invoking agent. Many
/// windows may exist, each pinned to its own agent; re-invoking for the same
/// agent replaces that agent's window, and invoking from a lasso pane closes
/// it (toggle).
pub(crate) fn open_pane() -> Result<()> {
    let herdr_run = |args: &[&str]| -> Option<String> {
        std::process::Command::new(herdr::herdr_bin())
            .args(args)
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    // a split needs a target pane: the pane focused at invocation, or our own
    let target = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|ctx| herdr::parse_plugin_context(&ctx).0)
        .or_else(|| std::env::var("HERDR_PANE_ID").ok());

    let live: Vec<String> = herdr_run(&["pane", "list"])
        .map(|out| herdr::parse_pane_ids_by_label(&out, "Lasso review"))
        .unwrap_or_default();
    // agent pane → lasso pane, so a re-open for the same agent replaces its window
    let map_path = state_dir().join("windows.json");
    let mut map: std::collections::BTreeMap<String, String> = std::fs::read(&map_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    map.retain(|_, pane| live.contains(pane));

    // invoked from a lasso pane: toggle it closed
    if let Some(t) = &target {
        if live.contains(t) {
            let _ = herdr_run(&["plugin", "pane", "close", t]);
            map.retain(|_, pane| pane != t);
            let _ = std::fs::write(&map_path, serde_json::to_vec_pretty(&map)?);
            return Ok(());
        }
        // this agent already has a window: replace it
        if let Some(old) = map.get(t).cloned() {
            let _ = herdr_run(&["plugin", "pane", "close", &old]);
        }
    }

    let mut cmd = std::process::Command::new(herdr::herdr_bin());
    cmd.args([
        "plugin",
        "pane",
        "open",
        "--plugin",
        "lasso",
        "--entrypoint",
        "review",
        "--placement",
        "split",
        "--direction",
        "right",
    ]);
    if let Some(t) = &target {
        cmd.args(["--target-pane", t]);
    } else if let Ok(ws) = std::env::var("HERDR_WORKSPACE_ID") {
        cmd.args(["--workspace", &ws]);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&out.stderr));
        std::process::exit(1);
    }
    if let (Some(t), Some(opened)) = (
        &target,
        herdr::parse_opened_pane_id(&String::from_utf8_lossy(&out.stdout)),
    ) {
        map.insert(t.clone(), opened);
        let _ = std::fs::create_dir_all(state_dir());
        let _ = std::fs::write(&map_path, serde_json::to_vec_pretty(&map)?);
    }
    Ok(())
}

pub(crate) fn state_dir() -> PathBuf {
    std::env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("lasso-state"))
}

/// Perform an Action's side effects; returns true when the app should quit.
pub(crate) fn apply_action(
    action: Action,
    app: &mut App,
    base: &mut DiffBase,
    file_view: &mut FileView,
    standalone: bool,
) -> bool {
    match action {
        Action::Quit => return true,
        Action::None => {}
        Action::Refresh => refresh(app, *base, standalone),
        Action::ToggleBase => {
            *base = match *base {
                DiffBase::Head => DiffBase::MergeBase,
                DiffBase::MergeBase => DiffBase::Head,
            };
            app.status = format!(
                "diff base: {}",
                if *base == DiffBase::Head {
                    "HEAD"
                } else {
                    "merge-base"
                }
            );
            refresh(app, *base, standalone);
        }
        Action::OpenFile { path } => {
            let (fv, model) = load_file_view(app, &path, standalone);
            *file_view = fv;
            app.set_view(&path, model);
        }
        Action::Submit { pane_id, text } => match herdr::agent_prompt(&pane_id, &text) {
            Ok(()) => {
                let key = app.agent_key();
                let n = app.store.comments(&key).len();
                app.store.clear(&key);
                app.status = format!("review sent to {pane_id} ({n} comments)");
            }
            Err(e) => app.status = format!("send failed: {e}"),
        },
    }
    false
}

pub(crate) fn to_char(code: KeyCode, in_modal: bool) -> Option<char> {
    match code {
        KeyCode::Char(c) => Some(c),
        KeyCode::Enter => Some('\n'),
        KeyCode::Esc => Some('\u{1b}'),
        KeyCode::Backspace => Some('\u{7f}'),
        KeyCode::Tab => Some('\t'),
        KeyCode::Down => (!in_modal).then_some('j'),
        KeyCode::Up => (!in_modal).then_some('k'),
        KeyCode::Left => (!in_modal).then_some('h'),
        KeyCode::Right => (!in_modal).then_some('l'),
        _ => None,
    }
}

pub(crate) fn refresh(app: &mut App, base: DiffBase, standalone: bool) {
    if !standalone {
        match herdr::agent_list() {
            Ok(agents) => app.set_agents(agents),
            Err(e) => app.status = format!("herdr: {e}"),
        }
    }
    let cwd = app
        .agent()
        .map(|a| PathBuf::from(&a.cwd))
        .or_else(|| std::env::current_dir().ok());
    let Some(cwd) = cwd else { return };
    match gitio::repo_root(&cwd) {
        Some(root) => {
            let cwd_canon = root.canonicalize().unwrap_or_else(|_| root.clone());
            let mut roots = vec![root.clone()];
            // repos the agent touched in this session, per its transcript
            for extra in transcript_repo_roots(app) {
                if extra.canonicalize().map(|c| c != cwd_canon).unwrap_or(true)
                    && !roots.contains(&extra)
                {
                    roots.push(extra);
                }
            }
            let mut groups: Vec<(app::RepoRef, Vec<crate::diff::FileDiff>)> = Vec::new();
            for (i, r) in roots.iter().enumerate() {
                match gitio::full_diff(r, base) {
                    Ok(files) => {
                        // extra repos with no pending changes add only noise
                        if i == 0 || !files.is_empty() {
                            groups.push((repo_ref(r, &groups), files));
                        }
                    }
                    Err(e) => app.status = format!("git: {e}"),
                }
            }
            app.set_multi_diff(groups);
            if app.all_files_mode {
                app.all_files = gitio::ls_files(&root).unwrap_or_default();
                app.selected_file = app
                    .selected_file
                    .min(app.tree_rows().len().saturating_sub(1));
            }
        }
        None => {
            app.set_diff(Vec::new());
            app.status = format!("not a git repo: {}", cwd.display());
        }
    }
}

/// Label a repo by its dir name, disambiguating duplicates with the parent dir.
fn repo_ref(
    root: &std::path::Path,
    taken: &[(app::RepoRef, Vec<crate::diff::FileDiff>)],
) -> app::RepoRef {
    let base = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = if taken.iter().any(|(r, _)| r.label == base) {
        let parent = root
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{parent}-{base}")
    } else {
        base
    };
    app::RepoRef {
        root: root.to_path_buf(),
        label,
    }
}

/// Git roots of the files this window's agent edited, from the Claude Code
/// transcripts of its project. Sessions rotate (/clear, restarts), so all
/// transcripts touched within the last 24h are aggregated, not just the
/// current session. Cached per file by mtime — transcripts get large.
fn transcript_repo_roots(app: &App) -> Vec<PathBuf> {
    use std::sync::{Mutex, OnceLock};
    type Cache = std::collections::HashMap<PathBuf, (std::time::SystemTime, Vec<PathBuf>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    const RECENT: Duration = Duration::from_secs(24 * 3600);

    let Some(agent) = app.agent() else {
        return Vec::new();
    };
    if agent.agent != "claude" {
        return Vec::new();
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let dir = transcript::project_dir(&home, &agent.cwd);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let now = std::time::SystemTime::now();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));

    let mut roots: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if now
            .duration_since(mtime)
            .map(|age| age > RECENT)
            .unwrap_or(false)
        {
            continue;
        }
        let cached = cache.lock().ok().and_then(|g| {
            g.get(&path)
                .filter(|(t, _)| *t == mtime)
                .map(|(_, r)| r.clone())
        });
        let file_roots = match cached {
            Some(r) => r,
            None => {
                let r: Vec<PathBuf> = std::fs::read_to_string(&path)
                    .map(|jsonl| {
                        gitio::group_by_repo(&transcript::edited_files(&jsonl))
                            .into_keys()
                            .collect()
                    })
                    .unwrap_or_default();
                if let Ok(mut guard) = cache.lock() {
                    guard.insert(path, (mtime, r.clone()));
                }
                r
            }
        };
        for r in file_roots {
            if !roots.contains(&r) {
                roots.push(r);
            }
        }
    }
    roots
}

/// Repo root the current agent's files live in.
pub(crate) fn repo_root_for(app: &App, standalone: bool) -> PathBuf {
    let cwd = if standalone {
        std::env::current_dir().unwrap_or_default()
    } else {
        app.agent()
            .map(|a| PathBuf::from(&a.cwd))
            .unwrap_or_default()
    };
    gitio::repo_root(&cwd).unwrap_or(cwd)
}

/// Lines of a repo file; empty when unreadable (deleted/binary files).
/// Lines of a file; empty when unreadable (deleted/binary files).
pub(crate) fn read_lines(full: &std::path::Path) -> Vec<String> {
    match std::fs::read_to_string(full) {
        Ok(content) => content.lines().map(str::to_string).collect(),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn load_file_view(
    app: &App,
    path: &str,
    standalone: bool,
) -> (FileView, Vec<(Option<u32>, String)>) {
    let file_idx = app.file_index_by_display(path);
    let (root, rel) = match file_idx {
        Some(i) => {
            let rel = app.files[i].new_path.clone();
            let root = app
                .file_repo
                .get(i)
                .and_then(|&r| app.repos.get(r))
                .map(|r| r.root.clone())
                .unwrap_or_else(|| repo_root_for(app, standalone));
            (root, rel)
        }
        None => (repo_root_for(app, standalone), path.to_string()),
    };
    let content = std::fs::read_to_string(root.join(&rel)).unwrap_or_else(|e| format!("<{e}>"));
    let file_diff = file_idx.and_then(|i| app.files.get(i));
    let changed: BTreeSet<u32> = file_diff
        .map(|f| {
            f.hunks
                .iter()
                .flat_map(|h| h.lines.iter())
                .filter(|l| l.kind == diff::LineKind::Add)
                .filter_map(|l| l.new_no)
                .collect()
        })
        .unwrap_or_default();
    let deletions = file_diff.map(diff::deletions_by_anchor).unwrap_or_default();

    let mut lines = Vec::new();
    let mut model: Vec<(Option<u32>, String)> = Vec::new();
    let plain: Vec<String> = content.lines().map(str::to_string).collect();
    for (i, (line, is_changed)) in highlight::highlight_file(path, &content, &changed)
        .into_iter()
        .enumerate()
    {
        let no = i as u32 + 1;
        if let Some(dels) = deletions.get(&no) {
            for t in dels {
                lines.push(ui::FvLine::Deleted { text: t.clone() });
                model.push((None, t.clone()));
            }
        }
        lines.push(ui::FvLine::Content {
            line,
            changed: is_changed,
        });
        model.push((Some(no), plain.get(i).cloned().unwrap_or_default()));
    }
    // deletions anchored past the last content line (EOF deletions)
    let n = lines
        .iter()
        .filter(|l| matches!(l, ui::FvLine::Content { .. }))
        .count() as u32;
    for (_, dels) in deletions.range(n + 1..) {
        for t in dels {
            lines.push(ui::FvLine::Deleted { text: t.clone() });
            model.push((None, t.clone()));
        }
    }
    (
        FileView {
            path: path.to_string(),
            lines,
        },
        model,
    )
}

pub(crate) fn spawn_socket_listener(tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let Some(socket_path) = std::env::var_os("HERDR_SOCKET_PATH") else {
            return;
        };
        let mut backoff = 1u64;
        loop {
            if listen_once(&socket_path, &tx).is_ok() {
                backoff = 1;
            }
            std::thread::sleep(Duration::from_secs(backoff));
            backoff = (backoff * 2).min(30);
        }
    });
}

fn listen_once(socket_path: &std::ffi::OsStr, tx: &mpsc::Sender<AppEvent>) -> Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let pane_ids: Vec<String> = herdr::agent_list()?
        .into_iter()
        .map(|a| a.pane_id)
        .collect();
    if pane_ids.is_empty() {
        return Ok(());
    }
    let mut stream = UnixStream::connect(socket_path)?;
    let refs: Vec<&str> = pane_ids.iter().map(String::as_str).collect();
    stream.write_all(herdr::subscribe_request_line("lasso-sub", &refs).as_bytes())?;
    let reader = std::io::BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if herdr::parse_event_line(&line).is_some() && tx.send(AppEvent::AgentsChanged).is_err() {
            return Ok(());
        }
    }
    Ok(())
}
