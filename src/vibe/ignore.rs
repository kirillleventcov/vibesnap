use crate::error::Result;
use crate::vibe::paths::is_reserved;
use std::fs;
use std::path::{Path, PathBuf};

/// Read and parse ignore patterns from .gitignore (preferred) or .vibeignore (fallback)
pub fn read_ignore_patterns(root: &Path) -> Result<Vec<String>> {
    // First check for .gitignore
    let gitignore_path: PathBuf = root.join(".gitignore");
    if gitignore_path.exists() {
        let content: String = fs::read_to_string(gitignore_path)?;
        let patterns: Vec<String> = content
            .lines()
            .map(|line: &str| line.trim())
            .filter(|line: &&str| !line.is_empty() && !line.starts_with('#'))
            .map(|line: &str| line.to_string())
            .collect();
        return Ok(patterns);
    }

    // Fall back to .vibeignore
    let vibeignore_path = root.join(".vibeignore");
    if !vibeignore_path.exists() {
        return Ok(Vec::new());
    }

    let content: String = fs::read_to_string(vibeignore_path)?;
    let patterns: Vec<String> = content
        .lines()
        .map(|line: &str| line.trim())
        .filter(|line: &&str| !line.is_empty() && !line.starts_with('#'))
        .map(|line: &str| line.to_string())
        .collect();

    Ok(patterns)
}

/// Check if a repo-relative path (with `/` separators) should be skipped.
/// `.vibe` and `.git` are always skipped, whatever the ignore files say.
pub fn is_ignored(relative: &str, patterns: &[String]) -> bool {
    if is_reserved(relative) {
        return true;
    }
    patterns
        .iter()
        .any(|pattern| matches_pattern(relative, pattern))
}

/// Simple pattern matching function supporting wildcards and directory patterns
fn matches_pattern(path: &str, pattern: &str) -> bool {
    if let Some(dir_pattern) = pattern.strip_suffix('/') {
        // Directory pattern - matches the directory and everything under it
        return path == dir_pattern || path.starts_with(&format!("{}/", dir_pattern));
    }

    if pattern.contains('*') {
        // Simple wildcard matching
        return matches_wildcard(path, pattern);
    }

    // Exact match
    path == pattern
}

fn matches_wildcard(text: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return text == pattern;
    }

    let mut current_pos: usize = 0;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            // First part must match from the beginning
            if !text[current_pos..].starts_with(part) {
                return false;
            }
            current_pos += part.len();
        } else if i == parts.len() - 1 {
            // Last part must match at the end
            return text[current_pos..].ends_with(part);
        } else {
            // Middle parts
            if let Some(pos) = text[current_pos..].find(part) {
                current_pos += pos + part.len();
            } else {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_dirs_are_always_ignored() {
        assert!(is_ignored(".vibe", &[]));
        assert!(is_ignored(".vibe/meta.db", &[]));
        assert!(is_ignored(".git/HEAD", &[]));
        assert!(!is_ignored(".vibeignore", &[]));
        assert!(!is_ignored(".github/workflows/ci.yml", &[]));
    }

    #[test]
    fn patterns_still_apply() {
        let patterns = vec!["node_modules/".to_string(), "*.log".to_string()];
        assert!(is_ignored("node_modules", &patterns));
        assert!(is_ignored("node_modules/a/b.js", &patterns));
        assert!(is_ignored("debug.log", &patterns));
        assert!(!is_ignored("src/main.rs", &patterns));
    }
}
