//! Shell-out wrappers around the system `git`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::diff::{self, DiffLine, FileDiff, FileStatus, Hunk, LineKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffBase {
    Head,
    MergeBase,
}

pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Full diff against the base, untracked files included as fully-added.
pub fn full_diff(root: &Path, base: DiffBase) -> Result<Vec<FileDiff>> {
    let base_ref = match base {
        DiffBase::Head => "HEAD".to_string(),
        DiffBase::MergeBase => merge_base(root).unwrap_or_else(|| "HEAD".to_string()),
    };
    let text = git(root, &["diff", "--no-color", "--find-renames", &base_ref])?;
    let mut files = diff::parse(&text);
    for path in untracked(root)? {
        files.push(synthesize_added(root, &path));
    }
    files.sort_by(|a, b| a.new_path.cmp(&b.new_path));
    Ok(files)
}

/// Tracked + untracked (gitignore honored), repo-relative, sorted.
pub fn ls_files(root: &Path) -> Result<Vec<String>> {
    let out = git(
        root,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )?;
    let mut files: Vec<String> = out.lines().map(str::to_string).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

fn merge_base(root: &Path) -> Option<String> {
    for main in ["main", "master"] {
        if let Ok(out) = git(root, &["merge-base", "HEAD", main]) {
            let sha = out.trim();
            if !sha.is_empty() {
                return Some(sha.to_string());
            }
        }
    }
    None
}

fn untracked(root: &Path) -> Result<Vec<String>> {
    let out = git(root, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(out.lines().map(str::to_string).collect())
}

fn synthesize_added(root: &Path, path: &str) -> FileDiff {
    let content = std::fs::read_to_string(root.join(path)).unwrap_or_default();
    let lines: Vec<DiffLine> = content
        .lines()
        .enumerate()
        .map(|(i, text)| DiffLine {
            kind: LineKind::Add,
            old_no: None,
            new_no: Some(i as u32 + 1),
            text: text.to_string(),
        })
        .collect();
    let hunks = if lines.is_empty() {
        Vec::new()
    } else {
        vec![Hunk {
            header: format!("@@ -0,0 +1,{} @@", lines.len()),
            lines,
        }]
    };
    FileDiff {
        old_path: path.to_string(),
        new_path: path.to_string(),
        status: FileStatus::Added,
        hunks,
    }
}

/// Group absolute file paths by the git repo containing them; paths outside
/// any repo are dropped. Keys are repo roots.
pub fn group_by_repo(
    paths: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeMap<PathBuf, Vec<String>> {
    let mut root_cache: std::collections::BTreeMap<PathBuf, Option<PathBuf>> =
        std::collections::BTreeMap::new();
    let mut out: std::collections::BTreeMap<PathBuf, Vec<String>> =
        std::collections::BTreeMap::new();
    for path in paths {
        let p = Path::new(path);
        let Some(dir) = p.parent() else { continue };
        let root = root_cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| repo_root(dir))
            .clone();
        if let Some(root) = root {
            out.entry(root).or_default().push(path.clone());
        }
    }
    out
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("failed to run git")?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(
                ok.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&ok.stderr)
            );
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn repo_root_finds_root_from_subdir() {
        let dir = init_repo();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let root = repo_root(&sub).unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn repo_root_none_outside_git() {
        let dir = tempfile::tempdir().unwrap();
        assert!(repo_root(dir.path()).is_none());
    }

    #[test]
    fn full_diff_sees_modified_and_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "one\nTWO\n").unwrap();
        std::fs::write(dir.path().join("new.txt"), "hello\nworld\n").unwrap();
        let files = full_diff(dir.path(), DiffBase::Head).unwrap();
        assert_eq!(files.len(), 2);
        let modified = files.iter().find(|f| f.new_path == "a.txt").unwrap();
        assert_eq!(modified.status, FileStatus::Modified);
        let untracked = files.iter().find(|f| f.new_path == "new.txt").unwrap();
        assert_eq!(untracked.status, FileStatus::Added);
        let lines = &untracked.hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].kind, LineKind::Add);
        assert_eq!(lines[0].new_no, Some(1));
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[1].new_no, Some(2));
    }

    #[test]
    fn full_diff_clean_repo_is_empty() {
        let dir = init_repo();
        assert!(full_diff(dir.path(), DiffBase::Head).unwrap().is_empty());
    }

    #[test]
    fn full_diff_sees_staged_changes_too() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "one\nstaged\n").unwrap();
        let _ = git(dir.path(), &["add", "a.txt"]);
        let files = full_diff(dir.path(), DiffBase::Head).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].new_path, "a.txt");
    }

    #[test]
    fn merge_base_diff_includes_branch_commits() {
        let dir = init_repo();
        let run = |args: &[&str]| git(dir.path(), args).unwrap();
        run(&["checkout", "-q", "-b", "feature"]);
        std::fs::write(dir.path().join("feat.txt"), "x\n").unwrap();
        run(&["add", "."]);
        let _ = Command::new("git")
            .args([
                "-C",
                dir.path().to_str().unwrap(),
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "feat",
            ])
            .output()
            .unwrap();
        // committed on branch: invisible vs HEAD, visible vs merge-base with main
        assert!(full_diff(dir.path(), DiffBase::Head).unwrap().is_empty());
        let files = full_diff(dir.path(), DiffBase::MergeBase).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].new_path, "feat.txt");
    }

    #[test]
    fn group_by_repo_buckets_and_drops_outsiders() {
        let a = init_repo();
        let b = init_repo();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(a.path().join("sub")).unwrap();
        let paths: std::collections::BTreeSet<String> = [
            a.path().join("a.txt"),
            a.path().join("sub/deep.txt"),
            b.path().join("a.txt"),
            outside.path().join("loose.txt"),
        ]
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
        let groups = group_by_repo(&paths);
        assert_eq!(groups.len(), 2, "outsider dropped: {groups:?}");
        let a_root = a.path().canonicalize().unwrap();
        let files = groups
            .iter()
            .find(|(r, _)| r.canonicalize().unwrap() == a_root)
            .map(|(_, f)| f.len())
            .unwrap();
        assert_eq!(files, 2, "both files of repo A grouped");
    }

    #[test]
    fn ls_files_honors_gitignore() {
        let dir = init_repo();
        std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "x").unwrap();
        std::fs::write(dir.path().join("visible.txt"), "x").unwrap();
        let files = ls_files(dir.path()).unwrap();
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"visible.txt".to_string()));
        assert!(files.contains(&".gitignore".to_string()));
        assert!(!files.contains(&"ignored.txt".to_string()));
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
    }
}
