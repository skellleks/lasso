//! Extracting the files an agent edited from its Claude Code transcript.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Absolute paths of files edited (Edit/Write/MultiEdit/NotebookEdit) in a
/// Claude Code JSONL transcript.
pub fn edited_files(jsonl: &str) -> BTreeSet<String> {
    const EDIT_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];
    let mut out = BTreeSet::new();
    for line in jsonl.lines() {
        // cheap pre-filter before JSON parsing: transcripts are large
        if !line.contains("\"tool_use\"") || !line.contains("file_path") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if !EDIT_TOOLS.contains(&name) {
                continue;
            }
            if let Some(path) = block
                .pointer("/input/file_path")
                .or_else(|| block.pointer("/input/notebook_path"))
                .and_then(|p| p.as_str())
            {
                out.insert(path.to_string());
            }
        }
    }
    out
}

/// Where Claude Code stores the transcript for a session started in `cwd`:
/// `~/.claude/projects/<munged cwd>/<session id>.jsonl`.
pub fn transcript_path(home: &std::path::Path, cwd: &str, session_id: &str) -> PathBuf {
    let munged: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude")
        .join("projects")
        .join(munged)
        .join(format!("{session_id}.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_edited_paths_from_tool_use_blocks() {
        let jsonl = r#"{"type":"user","message":{"content":"hi"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"ok"},{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a/src/main.rs","old_string":"x"}}]}}
not json at all
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/repo/b/README.md","content":"z"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"rm /repo/c/x"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/a/src/main.rs"}}]}}
"#;
        let files = edited_files(jsonl);
        assert_eq!(
            files.into_iter().collect::<Vec<_>>(),
            vec!["/repo/a/src/main.rs".to_string(), "/repo/b/README.md".to_string()],
            "dedup, Bash ignored, bad lines skipped"
        );
    }

    #[test]
    fn empty_or_garbage_transcript_is_empty() {
        assert!(edited_files("").is_empty());
        assert!(edited_files("garbage\n{}\n").is_empty());
    }

    #[test]
    fn transcript_path_munges_cwd_like_claude_code() {
        let p = transcript_path(
            std::path::Path::new("/Users/admin"),
            "/Users/admin/Documents/work/mono/APIDATA",
            "46cce160-53bb-4610-a054-a559ca33d696",
        );
        assert_eq!(
            p,
            PathBuf::from("/Users/admin/.claude/projects/-Users-admin-Documents-work-mono-APIDATA/46cce160-53bb-4610-a054-a559ca33d696.jsonl")
        );
        // dots and underscores are munged too
        let p = transcript_path(std::path::Path::new("/h"), "/a/b.c_d", "id");
        assert_eq!(p, PathBuf::from("/h/.claude/projects/-a-b-c-d/id.jsonl"));
    }
}
