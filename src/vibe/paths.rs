use crate::error::{AppError, Result};
use std::path::{Component, Path, PathBuf};

use super::constants::RESERVED_DIRS;

/// Resolves a path given on the command line against the current directory, the
/// way git does, and returns it relative to the repo root with `/` separators.
/// The empty string means the root itself.
pub fn resolve_user_path(root: &Path, user_path: &Path) -> Result<String> {
    let cwd = std::env::current_dir()?;
    resolve_user_path_from(root, &cwd, user_path)
}

fn resolve_user_path_from(root: &Path, cwd: &Path, user_path: &Path) -> Result<String> {
    let absolute = normalize(&cwd.join(user_path));
    // Resolve symlinks in the parent directory only: a symlinked file must keep its
    // own name, but a symlinked ancestor (e.g. /tmp -> /private/tmp) must not make
    // the path look like it is outside the repo.
    let absolute = match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => match parent.canonicalize() {
            Ok(parent) => parent.join(name),
            Err(_) => absolute,
        },
        _ => absolute,
    };
    let relative = absolute.strip_prefix(root).map_err(|_| {
        AppError::Usage(format!(
            "{} is outside the repository at {}",
            user_path.display(),
            root.display()
        ))
    })?;
    let relative = to_slash(relative);
    if is_reserved(&relative) {
        return Err(AppError::Usage(format!(
            "{} is inside {}, which vibesnap never tracks",
            user_path.display(),
            relative.split('/').next().unwrap_or_default()
        )));
    }
    Ok(relative)
}

pub fn resolve_user_paths(root: &Path, user_paths: &[PathBuf]) -> Result<Vec<String>> {
    user_paths
        .iter()
        .map(|p| resolve_user_path(root, p))
        .collect()
}

/// Lexically removes `.` and `..` components.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn to_slash(relative: &Path) -> String {
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// True for `.vibe`, `.git` and anything below them.
pub fn is_reserved(relative: &str) -> bool {
    let first = relative.split('/').next().unwrap_or_default();
    RESERVED_DIRS.contains(&first)
}

/// True if a manifest path may be written or deleted: relative, without `.` or
/// `..` components, and outside the reserved directories. Old snapshots can contain
/// `.vibe/meta.db`; restoring that would overwrite the history itself.
pub fn is_safe_manifest_path(relative: &str) -> bool {
    !relative.is_empty()
        && !is_reserved(relative)
        && relative
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// True if `path` equals one of the scope prefixes or lies below one. An empty
/// scope, or the empty prefix, covers everything.
pub fn in_scope(path: &str, scope: &[String]) -> bool {
    scope.is_empty()
        || scope.iter().any(|prefix| {
            prefix.is_empty()
                || path == prefix
                || (path.starts_with(prefix.as_str())
                    && path.as_bytes().get(prefix.len()) == Some(&b'/'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(root: &str, cwd: &str, user: &str) -> Result<String> {
        resolve_user_path_from(Path::new(root), Path::new(cwd), Path::new(user))
    }

    #[test]
    fn resolves_against_cwd() {
        assert_eq!(
            resolve("/nonexistent/r", "/nonexistent/r/src", "main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            resolve("/nonexistent/r", "/nonexistent/r/src", "../a.txt").unwrap(),
            "a.txt"
        );
        assert_eq!(
            resolve("/nonexistent/r", "/nonexistent/r/src", ".").unwrap(),
            "src"
        );
        assert_eq!(
            resolve("/nonexistent/r", "/nonexistent/r/src", "..").unwrap(),
            ""
        );
        assert_eq!(
            resolve("/nonexistent/r", "/elsewhere", "/nonexistent/r/x/y").unwrap(),
            "x/y"
        );
    }

    #[test]
    fn rejects_outside_and_reserved() {
        assert!(matches!(
            resolve("/nonexistent/r", "/nonexistent/r", "../x"),
            Err(AppError::Usage(_))
        ));
        assert!(matches!(
            resolve("/nonexistent/r", "/nonexistent/r", ".vibe/meta.db"),
            Err(AppError::Usage(_))
        ));
        assert!(matches!(
            resolve("/nonexistent/r", "/nonexistent/r/.git", "config"),
            Err(AppError::Usage(_))
        ));
    }

    #[test]
    fn manifest_path_safety() {
        assert!(is_safe_manifest_path("src/main.rs"));
        assert!(is_safe_manifest_path(".vibeignore"));
        assert!(!is_safe_manifest_path(".vibe/meta.db"));
        assert!(!is_safe_manifest_path(".git/config"));
        assert!(!is_safe_manifest_path("../escape"));
        assert!(!is_safe_manifest_path("/etc/passwd"));
        assert!(!is_safe_manifest_path("a//b"));
        assert!(!is_safe_manifest_path(""));
    }

    #[test]
    fn scope_matching() {
        let scope = vec!["src".to_string()];
        assert!(in_scope("src", &scope));
        assert!(in_scope("src/a.rs", &scope));
        assert!(!in_scope("srcx/a.rs", &scope));
        assert!(!in_scope("README.md", &scope));
        assert!(in_scope("anything", &[]));
        assert!(in_scope("anything", &["".to_string()]));
    }
}
