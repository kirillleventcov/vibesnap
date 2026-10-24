//! What is never recorded: `.vibe`, `.git`, nested repositories, ignored files,
//! likely secrets and files above the size cap.

use crate::config::Config;
use crate::paths::is_reserved;
use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Never recorded even when not ignored: they usually hold credentials.
pub const DEFAULT_SECRET_PATTERNS: &[&str] = &[".env", ".env.*", "*.pem", "*.key"];

pub const IGNORE_FILES: &[&str] = &[".gitignore", ".ignore", ".vibeignore"];

pub fn secret_matcher(root: &Path, config: &Config) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in DEFAULT_SECRET_PATTERNS
        .iter()
        .copied()
        .chain(config.secret_patterns.iter().map(String::as_str))
    {
        let _ = builder.add_line(None, pattern);
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Answers "is this one path recorded?" without walking the tree, for hooks.
/// Follows the same files the walker reads: `.gitignore`, `.ignore` and
/// `.vibeignore` in every directory down to the path, `.git/info/exclude` and
/// the global excludes file. Deeper files win.
pub struct Oracle {
    root: PathBuf,
    base: Vec<Gitignore>,
    secrets: Gitignore,
    per_dir: HashMap<PathBuf, Vec<Gitignore>>,
    max_size: u64,
}

impl Oracle {
    pub fn new(root: &Path, config: &Config, git_exclude: Option<PathBuf>) -> Oracle {
        let mut base = Vec::new();
        let (global, _) = Gitignore::global();
        base.push(global);
        if let Some(Ok(content)) = git_exclude.map(std::fs::read_to_string) {
            // The exclude file's patterns are relative to the work tree root.
            let mut builder = GitignoreBuilder::new(root);
            for line in content.lines() {
                let _ = builder.add_line(None, line);
            }
            base.push(builder.build().unwrap_or_else(|_| Gitignore::empty()));
        }
        Oracle {
            root: root.to_path_buf(),
            base,
            secrets: secret_matcher(root, config),
            per_dir: HashMap::new(),
            max_size: config.max_file_size,
        }
    }

    fn matchers_for(&mut self, dir: &Path) -> &Vec<Gitignore> {
        self.per_dir.entry(dir.to_path_buf()).or_insert_with(|| {
            IGNORE_FILES
                .iter()
                .map(|name| dir.join(name))
                .filter(|p| p.is_file())
                .map(|p| Gitignore::new(p).0)
                .collect()
        })
    }

    /// True if `rel` (a file path relative to the root) must not be recorded.
    pub fn is_excluded(&mut self, rel: &str) -> bool {
        if rel.is_empty() || is_reserved(rel) {
            return true;
        }
        let abs = self.root.join(rel);
        if self
            .secrets
            .matched_path_or_any_parents(&abs, false)
            .is_ignore()
        {
            return true;
        }
        // A directory on the way that is its own repository is never recorded.
        let mut dir = self.root.clone();
        let parts: Vec<&str> = rel.split('/').collect();
        let mut verdict: Match<()> = Match::None;
        for m in &self.base {
            let r = m.matched_path_or_any_parents(&abs, false);
            if !r.is_none() {
                verdict = r.map(|_| ());
            }
        }
        for (i, part) in parts.iter().enumerate() {
            if i > 0 && dir.join(".git").exists() {
                return true;
            }
            let matchers = self.matchers_for(&dir).clone();
            for m in &matchers {
                let r = m.matched_path_or_any_parents(&abs, false);
                if !r.is_none() {
                    verdict = r.map(|_| ());
                }
            }
            dir.push(part);
        }
        if verdict.is_ignore() {
            return true;
        }
        match std::fs::symlink_metadata(&abs) {
            Ok(meta) => meta.is_file() && meta.len() > self.max_size,
            Err(_) => false,
        }
    }
}
