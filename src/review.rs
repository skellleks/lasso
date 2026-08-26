//! Review comments: model, prompt composer, persistence.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub path: String,
    pub side: Side,
    pub line_no: u32,
    /// Surrounding diff lines, verbatim with +/-/space prefixes.
    pub quote: Vec<String>,
    pub text: String,
}

/// Comments grouped per agent key, persisted as JSON in the plugin state dir.
pub struct ReviewStore {
    path: PathBuf,
    by_agent: BTreeMap<String, Vec<Comment>>,
}

impl ReviewStore {
    pub fn load(state_dir: &std::path::Path) -> Self {
        let path = state_dir.join("comments.json");
        let by_agent = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, by_agent }
    }

    pub fn comments(&self, agent_key: &str) -> &[Comment] {
        self.by_agent
            .get(agent_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn add(&mut self, agent_key: &str, comment: Comment) {
        self.by_agent
            .entry(agent_key.to_string())
            .or_default()
            .push(comment);
        self.save();
    }

    pub fn remove(&mut self, agent_key: &str, index: usize) {
        if let Some(list) = self.by_agent.get_mut(agent_key) {
            if index < list.len() {
                list.remove(index);
            }
        }
        self.save();
    }

    pub fn clear(&mut self, agent_key: &str) {
        self.by_agent.remove(agent_key);
        self.save();
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&self.by_agent) {
            let _ = std::fs::write(&self.path, json);
        }
    }
}

pub fn compose_prompt(comments: &[Comment]) -> String {
    let mut out = String::from("Code review: comments on your uncommitted changes.\n");
    for c in comments {
        let side = match c.side {
            Side::Old => "old",
            Side::New => "new",
        };
        out.push_str(&format!("\n## {}:{} ({})\n", c.path, c.line_no, side));
        for q in &c.quote {
            out.push_str(&format!("> {}\n", q));
        }
        out.push_str(&format!("Comment: {}\n", c.text));
    }
    out.push_str("\nPlease address every item above.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Comment> {
        vec![
            Comment {
                path: "app/user.py".into(),
                side: Side::New,
                line_no: 42,
                quote: vec![
                    "-    return None".into(),
                    "+    user = repo.find(id)".into(),
                ],
                text: "should raise 404 here, not return None".into(),
            },
            Comment {
                path: "app/config.py".into(),
                side: Side::Old,
                line_no: 7,
                quote: vec!["-TIMEOUT = 5".into()],
                text: "why was the timeout removed?".into(),
            },
        ]
    }

    #[test]
    fn prompt_contains_paths_lines_quotes_and_texts() {
        let p = compose_prompt(&sample());
        assert!(p.contains("app/user.py:42"));
        assert!(p.contains("app/config.py:7"));
        assert!(p.contains("> +    user = repo.find(id)"));
        assert!(p.contains("should raise 404 here"));
        assert!(p.contains("why was the timeout removed?"));
        // review framing + call to action
        assert!(p.starts_with("Code review"));
        assert!(p.trim_end().ends_with("Please address every item above."));
    }

    #[test]
    fn prompt_marks_old_side_lines() {
        let p = compose_prompt(&sample());
        assert!(p.contains("app/user.py:42 (new)"));
        assert!(p.contains("app/config.py:7 (old)"));
    }

    #[test]
    fn store_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut s = ReviewStore::load(dir.path());
            s.add("w1:p1|/repo", sample()[0].clone());
            s.add("w1:p2|/other", sample()[1].clone());
        }
        let s = ReviewStore::load(dir.path());
        assert_eq!(s.comments("w1:p1|/repo").len(), 1);
        assert_eq!(s.comments("w1:p1|/repo")[0].text, sample()[0].text);
        assert_eq!(s.comments("w1:p2|/other").len(), 1);
    }

    #[test]
    fn remove_and_clear_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ReviewStore::load(dir.path());
        s.add("k", sample()[0].clone());
        s.add("k", sample()[1].clone());
        s.remove("k", 0);
        assert_eq!(s.comments("k").len(), 1);
        assert_eq!(s.comments("k")[0].path, "app/config.py");
        s.clear("k");
        let reloaded = ReviewStore::load(dir.path());
        assert!(reloaded.comments("k").is_empty());
    }

    #[test]
    fn unknown_agent_key_has_no_comments() {
        let dir = tempfile::tempdir().unwrap();
        let s = ReviewStore::load(dir.path());
        assert!(s.comments("nope").is_empty());
    }

    #[test]
    fn load_survives_corrupt_state_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("comments.json"), b"{oops").unwrap();
        let s = ReviewStore::load(dir.path());
        assert!(s.comments("k").is_empty());
    }
}
