//! Parser for `git diff` unified output.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
}

pub fn parse(input: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_no = 0u32;
    let mut new_no = 0u32;

    for line in input.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (old_path, new_path) = split_git_paths(rest);
            files.push(FileDiff {
                old_path,
                new_path,
                status: FileStatus::Modified,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };

        if line.starts_with("new file mode") {
            file.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = FileStatus::Deleted;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            file.status = FileStatus::Renamed;
            file.old_path = from.to_string();
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.new_path = to.to_string();
        } else if line.starts_with("Binary files ") {
            file.status = FileStatus::Binary;
        } else if line.starts_with("@@") {
            let (o, n) = parse_hunk_header(line);
            old_no = o;
            new_no = n;
            file.hunks.push(Hunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(hunk) = file.hunks.last_mut() {
            let (kind, text) = match line.as_bytes().first() {
                Some(b' ') => (LineKind::Context, &line[1..]),
                Some(b'+') => (LineKind::Add, &line[1..]),
                Some(b'-') => (LineKind::Del, &line[1..]),
                Some(b'\\') => continue, // "\ No newline at end of file"
                _ => continue,
            };
            let (o, n) = match kind {
                LineKind::Context => {
                    let r = (Some(old_no), Some(new_no));
                    old_no += 1;
                    new_no += 1;
                    r
                }
                LineKind::Del => {
                    let r = (Some(old_no), None);
                    old_no += 1;
                    r
                }
                LineKind::Add => {
                    let r = (None, Some(new_no));
                    new_no += 1;
                    r
                }
            };
            hunk.lines.push(DiffLine {
                kind,
                old_no: o,
                new_no: n,
                text: text.to_string(),
            });
        }
    }
    files
}

/// Deleted lines (full `DiffLine`s) grouped by the new-file line number they
/// precede. A deletion at end of file anchors to `last_new_no + 1`.
pub fn deleted_lines_by_anchor(file: &FileDiff) -> std::collections::BTreeMap<u32, Vec<DiffLine>> {
    let mut anchors: std::collections::BTreeMap<u32, Vec<DiffLine>> =
        std::collections::BTreeMap::new();
    for hunk in &file.hunks {
        let mut pending: Vec<DiffLine> = Vec::new();
        let mut last_new: Option<u32> = None;
        for line in &hunk.lines {
            match (&line.kind, line.new_no) {
                (LineKind::Del, _) => pending.push(line.clone()),
                (_, Some(n)) => {
                    if !pending.is_empty() {
                        anchors.entry(n).or_default().append(&mut pending);
                    }
                    last_new = Some(n);
                }
                _ => {}
            }
        }
        if !pending.is_empty() {
            // deletion with nothing after it: anchor right past the last new line
            let anchor = last_new
                .map(|l| l + 1)
                .unwrap_or_else(|| parse_hunk_header(&hunk.header).1 + 1);
            anchors.entry(anchor).or_default().append(&mut pending);
        }
    }
    anchors
}

/// Same, but only the deleted texts.
pub fn deletions_by_anchor(file: &FileDiff) -> std::collections::BTreeMap<u32, Vec<String>> {
    deleted_lines_by_anchor(file)
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().map(|l| l.text).collect()))
        .collect()
}

/// Half-open char ranges within a line.
pub type CharRanges = Vec<(usize, usize)>;

/// Char ranges (half-open, in chars) that differ between a deleted line and
/// the added line that replaced it, token-LCS based. Empty when the lines
/// share too little to highlight meaningfully.
pub fn word_diff_ranges(old: &str, new: &str) -> (CharRanges, CharRanges) {
    let ot = tokenize(old);
    let nt = tokenize(new);
    if ot.len() > 200 || nt.len() > 200 {
        return (Vec::new(), Vec::new());
    }
    // token LCS
    let (n, m) = (ot.len(), nt.len());
    let mut dp = vec![0u16; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * (m + 1) + j] = if ot[i].2 == nt[j].2 {
                dp[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                dp[(i + 1) * (m + 1) + j].max(dp[i * (m + 1) + j + 1])
            };
        }
    }
    let mut old_keep = vec![false; n];
    let mut new_keep = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if ot[i].2 == nt[j].2 {
            old_keep[i] = true;
            new_keep[j] = true;
            i += 1;
            j += 1;
        } else if dp[(i + 1) * (m + 1) + j] >= dp[i * (m + 1) + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    // gate: emphasize only when the lines genuinely share content
    let solid = |t: &(usize, usize, String)| !t.2.trim().is_empty();
    let matched = ot
        .iter()
        .zip(&old_keep)
        .filter(|(t, k)| **k && solid(t))
        .count();
    let base = ot
        .iter()
        .filter(|t| solid(t))
        .count()
        .max(nt.iter().filter(|t| solid(t)).count());
    if base > 0 && matched * 2 < base {
        return (Vec::new(), Vec::new());
    }
    (
        collect_ranges(&ot, &old_keep),
        collect_ranges(&nt, &new_keep),
    )
}

/// (char start, char end, text) tokens: identifier runs, whitespace runs,
/// single punctuation chars.
fn tokenize(s: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let c = chars[i];
        let same = |d: char| {
            if c.is_alphanumeric() || c == '_' {
                d.is_alphanumeric() || d == '_'
            } else if c.is_whitespace() {
                d.is_whitespace()
            } else {
                false
            }
        };
        i += 1;
        while i < chars.len() && same(chars[i]) {
            i += 1;
        }
        out.push((start, i, chars[start..i].iter().collect()));
    }
    out
}

fn collect_ranges(tokens: &[(usize, usize, String)], keep: &[bool]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (t, kept) in tokens.iter().zip(keep) {
        if *kept {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last.1 == t.0 => last.1 = t.1,
            _ => ranges.push((t.0, t.1)),
        }
    }
    // drop pure-whitespace ranges
    ranges.retain(|&(a, b)| {
        tokens
            .iter()
            .any(|t| t.0 >= a && t.1 <= b && !t.2.trim().is_empty())
    });
    ranges
}

/// "@@ -41,3 +41,4 @@ ..." → (41, 41)
fn parse_hunk_header(header: &str) -> (u32, u32) {
    let mut old = 0;
    let mut new = 0;
    for tok in header.split_whitespace() {
        if let Some(spec) = tok.strip_prefix('-') {
            old = spec
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(spec) = tok.strip_prefix('+') {
            new = spec
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            break;
        }
    }
    (old, new)
}

/// "a/path b/path" (paths may contain spaces) → (path, path)
fn split_git_paths(rest: &str) -> (String, String) {
    // Find the split point where "a/... b/..." divides: try every " b/" candidate
    // so paths with spaces still parse.
    for (idx, _) in rest.match_indices(" b/") {
        let old = &rest[..idx];
        let new = &rest[idx + 1..];
        if let (Some(o), Some(n)) = (old.strip_prefix("a/"), new.strip_prefix("b/")) {
            if o == n || !o.is_empty() {
                return (o.to_string(), n.to_string());
            }
        }
    }
    (rest.to_string(), rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODIFIED: &str = "\
diff --git a/app/user.py b/app/user.py
index 1234567..89abcde 100644
--- a/app/user.py
+++ b/app/user.py
@@ -41,3 +41,4 @@ def get_user(id):
 def get_user(id):
-    return None
+    user = repo.find(id)
+    return user
";

    #[test]
    fn parses_modified_file_with_line_numbers() {
        let files = parse(MODIFIED);
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.new_path, "app/user.py");
        assert_eq!(f.status, FileStatus::Modified);
        assert_eq!(f.hunks.len(), 1);
        let lines = &f.hunks[0].lines;
        assert_eq!(lines[0].kind, LineKind::Context);
        assert_eq!(lines[0].old_no, Some(41));
        assert_eq!(lines[0].new_no, Some(41));
        assert_eq!(lines[1].kind, LineKind::Del);
        assert_eq!(lines[1].old_no, Some(42));
        assert_eq!(lines[1].new_no, None);
        assert_eq!(lines[1].text, "    return None");
        assert_eq!(lines[2].kind, LineKind::Add);
        assert_eq!(lines[2].old_no, None);
        assert_eq!(lines[2].new_no, Some(42));
        assert_eq!(lines[3].kind, LineKind::Add);
        assert_eq!(lines[3].new_no, Some(43));
    }

    #[test]
    fn parses_added_and_deleted_files() {
        let input = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,1 @@
+hello
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index e69de29..0000000
--- a/gone.txt
+++ /dev/null
@@ -1,1 +0,0 @@
-bye
";
        let files = parse(input);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[0].new_path, "new.txt");
        assert_eq!(files[0].hunks[0].lines[0].new_no, Some(1));
        assert_eq!(files[1].status, FileStatus::Deleted);
        assert_eq!(files[1].old_path, "gone.txt");
    }

    #[test]
    fn parses_rename_with_similarity() {
        let input = "\
diff --git a/old_name.rs b/new_name.rs
similarity index 95%
rename from old_name.rs
rename to new_name.rs
index 1111111..2222222 100644
--- a/old_name.rs
+++ b/new_name.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old();
+    new();
";
        let files = parse(input);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].old_path, "old_name.rs");
        assert_eq!(files[0].new_path, "new_name.rs");
    }

    #[test]
    fn marks_binary_files_without_hunks() {
        let input = "\
diff --git a/logo.png b/logo.png
index 1111111..2222222 100644
Binary files a/logo.png and b/logo.png differ
";
        let files = parse(input);
        assert_eq!(files[0].status, FileStatus::Binary);
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn handles_no_newline_marker_and_multiple_hunks() {
        let input = "\
diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
-one
+uno
 two
@@ -10,2 +10,2 @@
 ten
-eleven
+once
\\ No newline at end of file
";
        let files = parse(input);
        assert_eq!(files[0].hunks.len(), 2);
        let h2 = &files[0].hunks[1];
        assert_eq!(h2.lines[0].old_no, Some(10));
        // marker line is not a diff line
        assert_eq!(h2.lines.len(), 3);
    }

    #[test]
    fn empty_input_yields_no_files() {
        assert!(parse("").is_empty());
    }

    #[test]
    fn deletions_anchor_to_following_new_line() {
        let files = parse(MODIFIED);
        // "-    return None" is followed by adds at new_no 42
        let anchors = deletions_by_anchor(&files[0]);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[&42], vec!["    return None".to_string()]);
    }

    #[test]
    fn deletion_at_eof_anchors_past_last_line() {
        let input = "\
diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,3 +1,1 @@
 one
-two
-three
";
        let files = parse(input);
        let anchors = deletions_by_anchor(&files[0]);
        assert_eq!(anchors[&2], vec!["two".to_string(), "three".to_string()]);
    }

    fn covered(ranges: &[(usize, usize)], s: &str, needle: &str) -> bool {
        let start = s.find(needle).unwrap();
        let end = start + needle.chars().count();
        ranges.iter().any(|&(a, b)| a <= start && end <= b)
    }

    #[test]
    fn word_diff_marks_replaced_token() {
        let (old_r, new_r) = word_diff_ranges("    return None", "    return user");
        assert!(covered(&old_r, "    return None", "None"), "{old_r:?}");
        assert!(covered(&new_r, "    return user", "user"), "{new_r:?}");
        assert!(
            !covered(&old_r, "    return None", "return"),
            "unchanged token not marked"
        );
    }

    #[test]
    fn word_diff_marks_insertion_only_on_new_side() {
        let (old_r, new_r) = word_diff_ranges("foo(a, b)", "foo(a, b, c)");
        assert!(old_r.is_empty(), "{old_r:?}");
        assert!(covered(&new_r, "foo(a, b, c)", "c"), "{new_r:?}");
    }

    #[test]
    fn word_diff_identical_lines_mark_nothing() {
        let (old_r, new_r) = word_diff_ranges("same line", "same line");
        assert!(old_r.is_empty() && new_r.is_empty());
    }

    #[test]
    fn word_diff_unrelated_lines_mark_nothing() {
        let (old_r, new_r) = word_diff_ranges("abc def", "xyz uvw qrs");
        assert!(
            old_r.is_empty() && new_r.is_empty(),
            "no noise on full rewrites"
        );
    }

    #[test]
    fn no_deletions_no_anchors() {
        let input = "\
diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,2 @@
 one
+two
";
        assert!(deletions_by_anchor(&parse(input)[0]).is_empty());
    }

    #[test]
    fn parses_quoted_paths_with_spaces() {
        let input = "\
diff --git a/my file.txt b/my file.txt
index 1111111..2222222 100644
--- a/my file.txt
+++ b/my file.txt
@@ -1,1 +1,1 @@
-a
+b
";
        let files = parse(input);
        assert_eq!(files[0].new_path, "my file.txt");
    }
}
