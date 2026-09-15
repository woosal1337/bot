use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexFileDiff {
    pub path: String,
    pub diff: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodexDiffDetail {
    pub old_string: String,
    pub old_line: usize,
    pub new_string: String,
    pub new_line: usize,
    pub context_before: String,
    pub context_after: String,
    pub line_prefix: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineKind {
    Context,
    Delete,
    Insert,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HunkLine {
    kind: LineKind,
    text: String,
}

pub fn parse_unified_diff(diff: &str) -> Vec<CodexDiffDetail> {
    let mut details = Vec::new();
    let mut current: Option<(usize, usize, Vec<HunkLine>)> = None;
    for line in diff.split_inclusive('\n') {
        if let Some((old_line, new_line)) = parse_hunk_header(line) {
            if let Some(hunk) = current.take().and_then(finish_hunk) {
                details.push(hunk);
            }
            current = Some((old_line, new_line, Vec::new()));
            continue;
        }
        let Some((_, _, lines)) = current.as_mut() else {
            continue;
        };
        let Some((prefix, text)) = line.split_at_checked(1) else {
            continue;
        };
        let kind = match prefix {
            " " => LineKind::Context,
            "-" => LineKind::Delete,
            "+" => LineKind::Insert,
            _ => continue,
        };
        lines.push(HunkLine {
            kind,
            text: text.to_owned(),
        });
    }
    if let Some(hunk) = current.and_then(finish_hunk) {
        details.push(hunk);
    }
    details
}

pub fn split_turn_diff(diff: &str) -> Vec<CodexFileDiff> {
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
        .into_iter()
        .filter_map(|diff| {
            let path = diff_path(&diff)?;
            Some(CodexFileDiff { path, diff })
        })
        .collect()
}

fn diff_path(diff: &str) -> Option<String> {
    let new_path = header_path(diff, "+++ ");
    if new_path.as_deref() != Some("/dev/null") {
        return new_path;
    }
    header_path(diff, "--- ").filter(|path| path != "/dev/null")
}

fn header_path(diff: &str, prefix: &str) -> Option<String> {
    let value = diff.lines().find_map(|line| line.strip_prefix(prefix))?;
    if value == "/dev/null" {
        return Some(value.to_owned());
    }
    let value = value
        .strip_prefix("a/")
        .or_else(|| value.strip_prefix("b/"))?;
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let body = line.strip_prefix("@@ -")?;
    let (old, body) = body.split_once(" +")?;
    let (new, _) = body.split_once(" @@")?;
    Some((parse_range_start(old)?, parse_range_start(new)?))
}

fn parse_range_start(value: &str) -> Option<usize> {
    value.split(',').next()?.parse().ok()
}

fn finish_hunk(
    (old_start, new_start, lines): (usize, usize, Vec<HunkLine>),
) -> Option<CodexDiffDetail> {
    let first = lines
        .iter()
        .position(|line| line.kind != LineKind::Context)?;
    let last = lines
        .iter()
        .rposition(|line| line.kind != LineKind::Context)?;
    let context_before = join_text(&lines[..first], |_| true);
    let context_after = join_text(&lines[last + 1..], |_| true);
    let body = &lines[first..=last];
    let old_string = join_text(body, |kind| kind != LineKind::Insert);
    let new_string = join_text(body, |kind| kind != LineKind::Delete);
    Some(CodexDiffDetail {
        old_string,
        old_line: old_start.saturating_add(first),
        new_string,
        new_line: new_start.saturating_add(first),
        context_before,
        context_after,
        line_prefix: String::new(),
    })
}

fn join_text(lines: &[HunkLine], include: impl Fn(LineKind) -> bool) -> String {
    lines
        .iter()
        .filter(|line| include(line.kind))
        .map(|line| line.text.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_unified_diff_hunk() {
        let details = parse_unified_diff("@@ -10,4 +10,4 @@\n before\n-old\n+new\n after\n");
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].old_line, 11);
        assert_eq!(details[0].new_line, 11);
        assert_eq!(details[0].old_string, "old\n");
        assert_eq!(details[0].new_string, "new\n");
        assert_eq!(details[0].context_before, "before\n");
        assert_eq!(details[0].context_after, "after\n");
    }

    #[test]
    fn parses_insertions_deletions_and_several_hunks() {
        let details = parse_unified_diff(
            "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1,2 @@\n same\n+added\n@@ -8,2 +9 @@\n-removed\n kept\n",
        );
        assert_eq!(details.len(), 2);
        assert_eq!(details[0].old_string, "");
        assert_eq!(details[0].new_string, "added\n");
        assert_eq!(details[0].old_line, 2);
        assert_eq!(details[0].new_line, 2);
        assert_eq!(details[1].old_string, "removed\n");
        assert_eq!(details[1].new_string, "");
        assert_eq!(details[1].old_line, 8);
        assert_eq!(details[1].new_line, 9);
    }

    #[test]
    fn ignores_headers_and_incomplete_hunks() {
        assert!(parse_unified_diff("diff --git a/a b/a\n--- a/a\n+++ b/a\n").is_empty());
        assert!(parse_unified_diff("@@ malformed @@\n-old\n+new\n").is_empty());
    }

    #[test]
    fn splits_aggregate_turn_diffs_by_file() {
        let files = split_turn_diff(
            "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/docs/new file.md b/docs/new file.md\n--- /dev/null\n+++ b/docs/new file.md\n@@ -0,0 +1 @@\n+text\n",
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[1].path, "docs/new file.md");
        assert!(files[0].diff.contains("-old\n+new"));
        assert!(files[1].diff.contains("+text"));
    }

    #[test]
    fn uses_the_old_path_for_deleted_files() {
        let files = split_turn_diff(
            "diff --git a/old.txt b/old.txt\n--- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\n",
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "old.txt");
    }

    #[test]
    fn rejects_text_without_a_portable_file_path() {
        assert!(split_turn_diff("not a unified diff").is_empty());
        assert!(split_turn_diff("--- /dev/null\n+++ /dev/null\n").is_empty());
    }
}
