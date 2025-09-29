use crate::error::Result;
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

/// Check if a path should be ignored based on .vibeignore patterns
pub fn should_ignore_path(path: &Path, root: &Path, patterns: &[String]) -> bool {
    let relative_path: &Path = match path.strip_prefix(root) {
        Ok(rel) => rel,
        Err(_) => return false, // If not under root, don't ignore
    };

    let path_str: std::borrow::Cow<'_, str> = relative_path.to_string_lossy();

    // Always ignore .git directory
    if path_str == ".git" || path_str.starts_with(".git/") {
        return true;
    }

    for pattern in patterns {
        if matches_pattern(&path_str, pattern) {
            return true;
        }
    }

    false
}

/// Simple pattern matching function supporting wildcards and directory patterns
fn matches_pattern(path: &str, pattern: &str) -> bool {
    if pattern.ends_with('/') {
        // Directory pattern - matches the directory and everything under it
        let dir_pattern: &str = &pattern[..pattern.len() - 1];
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
