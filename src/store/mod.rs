//! The `.vibe` store: a bare git object database written through gitoxide, the
//! ledger ref that holds one commit per event, and the SQLite index.

pub mod index;
pub mod ledger;
pub mod lock;
pub mod odb;
pub mod v1;

use crate::config::Config;
use crate::error::{AppError, Result, git_err};
use crate::fsutil::tmp_path;
use rusqlite::Connection;
use std::cell::OnceCell;
use std::fs;
use std::path::{Path, PathBuf};

pub const STORE_DIR: &str = ".vibe";
const VERSION_FILE: &str = "version";
const STORE_VERSION: &str = "2";

/// Where the project's own git repository keeps things, when there is one.
#[derive(Debug, Clone)]
pub struct GitInfo {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// False for SHA-256 repositories: then nothing is borrowed from `.git`.
    pub sha1: bool,
}

impl GitInfo {
    pub fn detect(root: &Path) -> Option<GitInfo> {
        let dot_git = root.join(".git");
        if !dot_git.exists() {
            return None;
        }
        let git_dir = if dot_git.is_dir() {
            dot_git
        } else {
            // A worktree: ".git" is a file with "gitdir: <path>".
            let content = fs::read_to_string(&dot_git).ok()?;
            let dir = content.strip_prefix("gitdir:")?.trim();
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                root.join(dir)
            }
        };
        let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
            Ok(rel) => {
                let rel = PathBuf::from(rel.trim());
                if rel.is_absolute() {
                    rel
                } else {
                    git_dir.join(rel)
                }
            }
            Err(_) => git_dir.clone(),
        };
        let config = fs::read_to_string(common_dir.join("config")).unwrap_or_default();
        let sha1 = !config.lines().any(|l| {
            l.trim()
                .replace(' ', "")
                .eq_ignore_ascii_case("objectformat=sha256")
        });
        Some(GitInfo {
            git_dir,
            common_dir,
            sha1,
        })
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.common_dir.join("objects")
    }

    pub fn index_file(&self) -> PathBuf {
        self.git_dir.join("index")
    }

    pub fn exclude_file(&self) -> PathBuf {
        self.common_dir.join("info").join("exclude")
    }
}

pub struct Store {
    /// Canonical path of the project directory that contains `.vibe`.
    pub root: PathBuf,
    pub vibe: PathBuf,
    pub db: Connection,
    pub config: Config,
    repo: OnceCell<gix::Repository>,
    git: OnceCell<Option<GitInfo>>,
}

impl Store {
    /// Finds the nearest directory at or above `start` that has a `.vibe` store.
    pub fn discover(start: &Path) -> Option<PathBuf> {
        let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        start
            .ancestors()
            .find(|dir| dir.join(STORE_DIR).is_dir())
            .map(Path::to_path_buf)
    }

    /// Opens the store for the current directory.
    pub fn find() -> Result<Store> {
        let cwd = std::env::current_dir()?;
        let root = Store::discover(&cwd).ok_or(AppError::NotInRepo)?;
        Store::open(&root)
    }

    pub fn open(root: &Path) -> Result<Store> {
        let root = root.canonicalize()?;
        let vibe = root.join(STORE_DIR);
        if !vibe.join(VERSION_FILE).exists() {
            if vibe.join("meta.db").exists() {
                v1::migrate(&root)?;
            } else {
                return Err(AppError::NotInRepo);
            }
        }
        let db = index::connect(&vibe)?;
        Ok(Store {
            root,
            vibe,
            db,
            config: Config::load(),
            repo: OnceCell::new(),
            git: OnceCell::new(),
        })
    }

    /// Creates a store at `root` and records the current tree as the baseline
    /// event. If another process created one first, opens that one.
    pub fn init(root: &Path) -> Result<Store> {
        let root = root.canonicalize()?;
        let vibe = root.join(STORE_DIR);
        if vibe.exists() {
            return Store::open(&root);
        }
        let git = GitInfo::detect(&root);
        let tmp = tmp_path(&root, ".vibe-init-");
        let result = (|| -> Result<()> {
            create_layout(&tmp, git.as_ref())?;
            match fs::rename(&tmp, &vibe) {
                Ok(()) => Ok(()),
                // Someone else won the race; theirs is just as good.
                Err(_) if vibe.join(VERSION_FILE).exists() => Ok(()),
                Err(e) => Err(e.into()),
            }
        })();
        let _ = fs::remove_dir_all(&tmp);
        result?;
        if let Some(git) = &git {
            add_to_git_exclude(git)?;
        }
        let mut store = Store::open(&root)?;
        let lock = lock::lock(&store.vibe)?;
        crate::wc::scan::reconcile(
            &mut store,
            &lock,
            &ledger::EventMeta::cause("baseline", "user"),
        )?;
        drop(lock);
        Ok(store)
    }

    /// The object database, opened on first use.
    pub fn repo(&self) -> Result<&gix::Repository> {
        if let Some(repo) = self.repo.get() {
            return Ok(repo);
        }
        let repo = gix::open_opts(&self.vibe, gix::open::Options::isolated()).map_err(git_err)?;
        Ok(self.repo.get_or_init(|| repo))
    }

    pub fn git(&self) -> Option<&GitInfo> {
        self.git
            .get_or_init(|| GitInfo::detect(&self.root))
            .as_ref()
    }

    pub fn hook_log(&self) -> PathBuf {
        self.vibe.join("hook.log")
    }
}

pub(crate) fn create_layout(dir: &Path, git: Option<&GitInfo>) -> Result<()> {
    gix::init_bare(dir).map_err(git_err)?;
    // `git --git-dir=.vibe log` shows the ledger without naming it.
    fs::write(dir.join("HEAD"), "ref: refs/vibesnap/ledger\n")?;
    let _ = fs::remove_dir_all(dir.join("hooks"));
    let mut config = fs::read_to_string(dir.join("config")).unwrap_or_default();
    config.push_str("[gc]\n\tauto = 0\n[core]\n\tlogAllRefUpdates = false\n");
    fs::write(dir.join("config"), config)?;
    if let Some(git) = git.filter(|g| g.sha1 && g.objects_dir().is_dir()) {
        let info = dir.join("objects").join("info");
        fs::create_dir_all(&info)?;
        fs::write(
            info.join("alternates"),
            format!("{}\n", git.objects_dir().display()),
        )?;
    }
    drop(index::connect(dir)?);
    fs::write(dir.join(VERSION_FILE), format!("{STORE_VERSION}\n"))?;
    Ok(())
}

pub(crate) fn add_to_git_exclude(git: &GitInfo) -> Result<()> {
    let path = git.exclude_file();
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.trim() == ".vibe/" || l.trim() == ".vibe" || l.trim() == "/.vibe/")
    {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(".vibe/\n");
    fs::write(path, content)?;
    Ok(())
}
