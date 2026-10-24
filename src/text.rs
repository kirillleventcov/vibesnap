//! Text helpers: binary detection, line counts, unified diffs.

use similar::{ChangeTag, DiffOp, TextDiff};

/// Git's heuristic: a NUL byte in the first 8 KB.
pub fn is_binary(content: &[u8]) -> bool {
    content.iter().take(8000).any(|&b| b == 0)
}

/// Lines added and removed going from `old` to `new`, or None for binary content.
pub fn line_counts(old: &[u8], new: &[u8]) -> Option<(i64, i64)> {
    if is_binary(old) || is_binary(new) {
        return None;
    }
    let old = String::from_utf8_lossy(old);
    let new = String::from_utf8_lossy(new);
    let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
    let (mut added, mut removed) = (0, 0);
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            match change.tag() {
                ChangeTag::Insert => added += 1,
                ChangeTag::Delete => removed += 1,
                ChangeTag::Equal => {}
            }
        }
    }
    Some((added, removed))
}

/// A git-style unified diff with `context` lines around each hunk.
pub fn unified(
    old: &[u8],
    new: &[u8],
    path: &str,
    old_label: &str,
    new_label: &str,
    context: usize,
) -> String {
    let old = String::from_utf8_lossy(old);
    let new = String::from_utf8_lossy(new);
    let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
    let mut out = diff
        .unified_diff()
        .context_radius(context)
        .header(
            &format!("a/{path}\t{old_label}"),
            &format!("b/{path}\t{new_label}"),
        )
        .missing_newline_hint(true)
        .to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Line-level operations between two texts, as `(old_range, new_range, kind)`.
/// `kind` is 'e' for equal, 'd' delete, 'i' insert, 'r' replace.
pub fn line_ops(
    old: &str,
    new: &str,
) -> Vec<(std::ops::Range<usize>, std::ops::Range<usize>, char)> {
    let diff = TextDiff::from_lines(old, new);
    diff.ops()
        .iter()
        .map(|op| match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => (old_index..old_index + len, new_index..new_index + len, 'e'),
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index..old_index + old_len, new_index..new_index, 'd'),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index..old_index, new_index..new_index + new_len, 'i'),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index..old_index + old_len,
                new_index..new_index + new_len,
                'r',
            ),
        })
        .collect()
}

pub fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Human-readable size.
pub fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
