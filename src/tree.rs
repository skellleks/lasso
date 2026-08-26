//! File tree for the all-files mode.

use std::collections::BTreeSet;

/// One visible row of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: usize,
    pub name: String,
    /// Repo-relative path ("src" for the dir, "src/main.rs" for the file).
    pub path: String,
    pub is_dir: bool,
    pub expanded: bool,
}

/// Flatten sorted `paths` into visible rows, honoring collapsed dirs.
/// Dirs come before files on each level, both alphabetical.
pub fn visible_rows(paths: &[String], collapsed: &BTreeSet<String>) -> Vec<TreeRow> {
    #[derive(Default)]
    struct Node {
        dirs: std::collections::BTreeMap<String, Node>,
        files: BTreeSet<String>,
    }

    let mut root = Node::default();
    for path in paths {
        let mut node = &mut root;
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                node = node.dirs.entry(part.to_string()).or_default();
            } else {
                node.files.insert(part.to_string());
            }
        }
    }

    fn walk(node: &Node, prefix: &str, depth: usize, collapsed: &BTreeSet<String>, out: &mut Vec<TreeRow>) {
        for (name, child) in &node.dirs {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let expanded = !collapsed.contains(&path);
            out.push(TreeRow { depth, name: name.clone(), path: path.clone(), is_dir: true, expanded });
            if expanded {
                walk(child, &path, depth + 1, collapsed, out);
            }
        }
        for name in &node.files {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            out.push(TreeRow { depth, name: name.clone(), path, is_dir: false, expanded: false });
        }
    }

    let mut out = Vec::new();
    walk(&root, "", 0, collapsed, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        ["src/main.rs", "src/ui/mod.rs", "README.md", "src/app.rs", "Cargo.toml"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn builds_tree_dirs_first_alphabetical() {
        let rows = visible_rows(&paths(), &BTreeSet::new());
        let names: Vec<(usize, &str, bool)> =
            rows.iter().map(|r| (r.depth, r.name.as_str(), r.is_dir)).collect();
        assert_eq!(
            names,
            vec![
                (0, "src", true),
                (1, "ui", true),
                (2, "mod.rs", false),
                (1, "app.rs", false),
                (1, "main.rs", false),
                (0, "Cargo.toml", false),
                (0, "README.md", false),
            ]
        );
        assert_eq!(rows[2].path, "src/ui/mod.rs");
        assert_eq!(rows[1].path, "src/ui");
    }

    #[test]
    fn collapsed_dir_hides_children() {
        let collapsed = BTreeSet::from(["src".to_string()]);
        let rows = visible_rows(&paths(), &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "Cargo.toml", "README.md"]);
        assert!(!rows[0].expanded);
    }

    #[test]
    fn collapsed_subdir_keeps_siblings() {
        let collapsed = BTreeSet::from(["src/ui".to_string()]);
        let rows = visible_rows(&paths(), &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "ui", "app.rs", "main.rs", "Cargo.toml", "README.md"]);
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(visible_rows(&[], &BTreeSet::new()).is_empty());
    }
}
