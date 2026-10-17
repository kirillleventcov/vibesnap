use crate::error::{AppError, Result};
use std::path::{Component, Path, PathBuf};

/// Directory names that are never recorded, restored into or deleted, at any depth.
pub const RESERVED_DIRS: &[&str] = &[".vibe", ".git"];

/// Resolves a path given on the command line against the current directory, the
/// way git does, and returns it relative to the root with `/` separators. The
/// empty string means the root itself.
pub fn resolve_user_path(root: &Path, user_path: &Path) -> Result<String> {
    let cwd = std::env::current_dir()?;
    resolve_from(root, &cwd, user_path)
}

pub fn resolve_from(root: &Path, cwd: &Path, user_path: &Path) -> Result<String> {
    let lexical = normalize(&cwd.join(user_path));
    // Inside the root, take the path as written: resolving a symlinked
    // directory there would silently act on its target instead. Only a path
    // that reaches the root through a symlink (/tmp -> /private/tmp) is resolved.
    let absolute = if lexical.starts_with(root) {
        lexical
    } else {
        match (lexical.parent(), lexical.file_name()) {
            (Some(parent), Some(name)) => match parent.canonicalize() {
                Ok(parent) => parent.join(name),
                Err(_) => lexical,
            },
            _ => lexical,
        }
    };
    let relative = absolute.strip_prefix(root).map_err(|_| {
        AppError::Usage(format!(
            "{} is outside the project at {}",
            user_path.display(),
            root.display()
        ))
    })?;
    let relative = to_slash(relative);
    if is_reserved(&relative) {
        return Err(AppError::Usage(format!(
            "{} is inside .vibe or .git, which vibesnap never records",
            user_path.display()
        )));
    }
    Ok(relative)
}

/// Lexically removes `.` and `..` components.
pub fn normalize(path: &Path) -> PathBuf {
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

/// True if any component is `.vibe` or `.git`.
pub fn is_reserved(relative: &str) -> bool {
    relative
        .split('/')
        .any(|part| RESERVED_DIRS.contains(&part))
}

/// True if a recorded path may be written or deleted: relative, without `.` or
/// `..` components, and outside the reserved directories.
pub fn is_safe_path(relative: &str) -> bool {
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
        resolve_from(Path::new(root), Path::new(cwd), Path::new(user))
    }

    #[test]
    fn resolves_against_cwd() {
        assert_eq!(
            resolve("/nx/r", "/nx/r/src", "main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(resolve("/nx/r", "/nx/r/src", "../a.txt").unwrap(), "a.txt");
        assert_eq!(resolve("/nx/r", "/nx/r/src", "..").unwrap(), "");
        assert_eq!(resolve("/nx/r", "/elsewhere", "/nx/r/x/y").unwrap(), "x/y");
        assert!(matches!(
            resolve("/nx/r", "/nx/r", "../x"),
            Err(AppError::Usage(_))
        ));
        assert!(matches!(
            resolve("/nx/r", "/nx/r", ".vibe/index.db"),
            Err(AppError::Usage(_))
        ));
    }

    #[test]
    fn path_safety_and_scope() {
        assert!(is_safe_path("src/main.rs"));
        assert!(is_safe_path(".vibeignore"));
        assert!(!is_safe_path(".vibe/index.db"));
        assert!(!is_safe_path("sub/.git/config"));
        assert!(!is_safe_path("../escape"));
        assert!(!is_safe_path("a//b"));
        let scope = vec!["src".to_string()];
        assert!(in_scope("src/a.rs", &scope));
        assert!(!in_scope("srcx/a.rs", &scope));
        assert!(in_scope("x", &[]));
    }
}
