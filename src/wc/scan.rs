//! Reading the working tree cheaply. A stat cache (mtime, ctime, size, inode,
//! mode and blob ID per path) lets a scan skip reading files that have not
//! changed. A file only counts as clean if its stat matches and it was not
//! modified at or after the stamp taken when its entry was recorded (the
//! "racy git" rule); otherwise it is read and hashed again.

use super::filter::{Oracle, secret_matcher};
use crate::error::Result;
use crate::store::Store;
use crate::store::ledger::{self, CacheRow, EventMeta, FileChange};
use crate::store::lock::WriteLock;
use crate::store::odb::{self, Entry, MODE_EXEC, MODE_FILE, MODE_LINK};
use ignore::{WalkBuilder, WalkState};
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub size: i64,
    pub ino: i64,
    /// Modified at or after the scan stamp: do not trust this entry next time.
    pub racy: bool,
}

impl FileStat {
    pub fn of(meta: &Metadata, stamp_ns: i64) -> FileStat {
        let mtime_ns = meta.mtime() * 1_000_000_000 + meta.mtime_nsec();
        FileStat {
            mtime_ns,
            ctime_ns: meta.ctime() * 1_000_000_000 + meta.ctime_nsec(),
            size: meta.len() as i64,
            ino: meta.ino() as i64,
            racy: mtime_ns >= stamp_ns,
        }
    }

    fn same_file_state(&self, other: &FileStat) -> bool {
        self.mtime_ns == other.mtime_ns
            && self.ctime_ns == other.ctime_ns
            && self.size == other.size
            && self.ino == other.ino
    }
}

pub fn mode_of(meta: &Metadata) -> u32 {
    if meta.file_type().is_symlink() {
        MODE_LINK
    } else if meta.mode() & 0o111 != 0 {
        MODE_EXEC
    } else {
        MODE_FILE
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cached {
    pub stat: FileStat,
    pub entry: Entry,
}

pub fn load_cache(store: &Store) -> Result<HashMap<String, Cached>> {
    let count: usize = store.db.query_row(
        "SELECT COUNT(*) FROM stat_cache WHERE lane = 'main'",
        [],
        |r| r.get(0),
    )?;
    let mut cache = HashMap::with_capacity(count);
    let mut stmt = store.db.prepare_cached(
        "SELECT path, mtime_ns, ctime_ns, size, ino, mode, oid, racy FROM stat_cache WHERE lane = 'main'",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let oid = gix::ObjectId::from_hex(r.get_ref(6)?.as_bytes()?)
            .map_err(|e| crate::error::msg(e.to_string()))?;
        cache.insert(
            r.get_ref(0)?.as_str()?.to_string(),
            Cached {
                stat: FileStat {
                    mtime_ns: r.get(1)?,
                    ctime_ns: r.get(2)?,
                    size: r.get(3)?,
                    ino: r.get(4)?,
                    racy: r.get(7)?,
                },
                entry: Entry {
                    oid,
                    mode: r.get(5)?,
                },
            },
        );
    }
    Ok(cache)
}

pub fn cached(store: &Store, path: &str) -> Result<Option<Cached>> {
    use rusqlite::OptionalExtension;
    let row = store
        .db
        .query_row(
            "SELECT mtime_ns, ctime_ns, size, ino, mode, oid, racy FROM stat_cache WHERE lane = 'main' AND path = ?1",
            [path],
            |r| {
                Ok((
                    FileStat {
                        mtime_ns: r.get(0)?,
                        ctime_ns: r.get(1)?,
                        size: r.get(2)?,
                        ino: r.get(3)?,
                        racy: r.get(6)?,
                    },
                    r.get::<_, u32>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    match row {
        Some((stat, mode, oid)) => Ok(Some(Cached {
            stat,
            entry: Entry {
                oid: odb::parse_oid(&oid)?,
                mode,
            },
        })),
        None => Ok(None),
    }
}

/// Touches `.vibe/stamp` and returns its mtime, read back from the filesystem so
/// it has the same granularity as the files' own timestamps.
pub fn touch_stamp(vibe: &Path) -> Result<i64> {
    let path = vibe.join("stamp");
    let mut file = fs::File::create(&path)?;
    file.write_all(b"s")?;
    drop(file);
    let meta = fs::metadata(&path)?;
    Ok(meta.mtime() * 1_000_000_000 + meta.mtime_nsec())
}

/// Every recordable file under the root, with its lstat. Ignored directories,
/// `.vibe`, `.git`, nested repositories and Claude Code's worktrees are pruned
/// or dropped, not recorded.
pub fn walk(root: &Path, config: &crate::config::Config) -> Vec<(String, Metadata)> {
    let root = root.to_path_buf();
    let secrets = secret_matcher(&root, config);
    let max_size = config.max_file_size;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let out: Mutex<Vec<(String, Metadata)>> = Mutex::new(Vec::new());
    // Directories that turn out to be repositories of their own. Checking every
    // directory for a `.git` up front would cost one failed stat per directory,
    // so they are noticed when their `.git` entry is reached and dropped after.
    let nested: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let worktrees = root.join(".claude").join("worktrees");

    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(false)
        .parents(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .threads(threads)
        .add_custom_ignore_filename(".vibeignore");
    {
        let sink = nested.clone();
        builder.filter_entry(move |entry| {
            let name = entry.file_name().as_bytes();
            if name == b".git" {
                if entry.depth() > 1
                    && let Some(parent) = entry.path().parent()
                {
                    sink.lock().unwrap().push(parent.to_path_buf());
                }
                return false;
            }
            name != b".vibe" && entry.path() != worktrees
        });
    }
    builder.build_parallel().run(|| {
        let mut sink = Sink {
            local: Vec::new(),
            out: &out,
        };
        let root = &root;
        let secrets = &secrets;
        Box::new(move |result| {
            if let Ok(entry) = result
                && entry
                    .file_type()
                    .is_some_and(|t| t.is_file() || t.is_symlink())
                && let Ok(meta) = entry.metadata()
                && let Ok(rel) = entry.path().strip_prefix(root)
                && let Some(rel) = rel.to_str()
                && !secrets
                    .matched_path_or_any_parents(entry.path(), false)
                    .is_ignore()
                && !(meta.is_file() && meta.len() > max_size)
            {
                sink.local.push((rel.to_string(), meta));
            }
            WalkState::Continue
        })
    });
    let mut files = out.into_inner().unwrap();
    let nested: Vec<String> = nested
        .lock()
        .unwrap()
        .iter()
        .filter_map(|d| {
            d.strip_prefix(&root)
                .ok()
                .and_then(|r| r.to_str())
                .map(|r| format!("{r}/"))
        })
        .collect();
    if !nested.is_empty() {
        files.retain(|(rel, _)| !nested.iter().any(|n| rel.starts_with(n.as_str())));
    }
    files
}

/// Per-thread buffer that hands its entries over when the walker drops it.
struct Sink<'a> {
    local: Vec<(String, Metadata)>,
    out: &'a Mutex<Vec<(String, Metadata)>>,
}

impl Drop for Sink<'_> {
    fn drop(&mut self) {
        self.out.lock().unwrap().append(&mut self.local);
    }
}

/// Reads the bytes vibesnap stores for a path: file content, or a symlink's target.
pub fn read_recorded(path: &Path, meta: &Metadata) -> std::io::Result<Vec<u8>> {
    if meta.file_type().is_symlink() {
        Ok(fs::read_link(path)?.as_os_str().as_bytes().to_vec())
    } else {
        fs::read(path)
    }
}

/// What a scan found.
pub struct ScanResult {
    pub changes: Vec<FileChange>,
    pub cache_rows: Vec<CacheRow>,
    pub files: usize,
    pub hashed: usize,
    /// Files that could not be read; their recorded state may be stale.
    pub unreadable: Vec<String>,
}

/// Compares the working tree with the stat cache. Paths in `skip` are left out
/// entirely (their change belongs to a pending tool call). With `write`, new
/// content is stored; otherwise only hashed.
pub fn scan(store: &Store, skip: &HashSet<String>, write: bool) -> Result<ScanResult> {
    let trace = std::env::var_os("VIBESNAP_TRACE").is_some();
    let t0 = std::time::Instant::now();
    let stamp_ns = touch_stamp(&store.vibe)?;
    // Walk on other threads while this one loads the stat cache.
    let (root, config) = (&store.root, &store.config);
    let (files, cache) = std::thread::scope(|scope| {
        let walker = scope.spawn(move || walk(root, config));
        let cache = load_cache(store);
        (walker.join().unwrap_or_default(), cache)
    });
    let cache = cache?;
    if trace {
        eprintln!(
            "trace: walk and cache {:?} ({} files)",
            t0.elapsed(),
            files.len()
        );
    }

    let mut seen: HashSet<&str> = HashSet::with_capacity(files.len());
    let mut to_hash: Vec<(&str, &Metadata, FileStat, u32)> = Vec::new();
    let mut cache_rows = Vec::new();
    for (rel, meta) in &files {
        if skip.contains(rel) {
            continue;
        }
        seen.insert(rel);
        let stat = FileStat::of(meta, stamp_ns);
        let mode = mode_of(meta);
        match cache.get(rel) {
            Some(c) if !c.stat.racy && c.stat.same_file_state(&stat) && c.entry.mode == mode => {}
            _ => to_hash.push((rel, meta, stat, mode)),
        }
    }

    // Files git already hashed (same stat as in .git/index) need not be read.
    let mut known: HashMap<String, Entry> = HashMap::new();
    if to_hash.len() > 64 && seeding_is_exact(store, &files) {
        let seeds = git_index_seeds(store);
        for (rel, _, stat, mode) in &to_hash {
            if let Some((seed_stat, oid, seed_mode)) = seeds.get(*rel)
                && *seed_mode == *mode
                && seed_stat.0 == stat.mtime_ns
                && seed_stat.1 == stat.ctime_ns
                && seed_stat.2 == (stat.size as u32)
                && seed_stat.3 == (stat.ino as u32)
            {
                known.insert(
                    rel.to_string(),
                    Entry {
                        oid: *oid,
                        mode: *mode,
                    },
                );
            }
        }
        if !known.is_empty() {
            let repo = store.repo()?;
            known.retain(|_, e| odb::has_object(repo, e.oid));
        }
    }

    if trace {
        eprintln!(
            "trace: seeded {} from the git index {:?}",
            known.len(),
            t0.elapsed()
        );
    }
    let need: Vec<(&str, &Metadata, u32)> = to_hash
        .iter()
        .filter(|(rel, ..)| !known.contains_key(*rel))
        .map(|(rel, meta, _, mode)| (*rel, *meta, *mode))
        .collect();
    let hashed_count = need.len();
    let hashed = hash_files(store, &need, write)?;
    known.extend(hashed);

    let mut changes = Vec::new();
    let mut unreadable = Vec::new();
    for (rel, _, stat, _) in &to_hash {
        let Some(entry) = known.get(*rel) else {
            unreadable.push(rel.to_string());
            continue;
        };
        let before = cache.get(*rel).map(|c| c.entry);
        if before != Some(*entry) {
            changes.push(FileChange {
                path: rel.to_string(),
                before,
                after: Some(*entry),
            });
        }
        cache_rows.push(CacheRow {
            path: rel.to_string(),
            value: Some((*stat, *entry)),
        });
    }
    for (path, c) in &cache {
        if !seen.contains(path.as_str()) && !skip.contains(path) {
            changes.push(FileChange {
                path: path.clone(),
                before: Some(c.entry),
                after: None,
            });
            cache_rows.push(CacheRow {
                path: path.clone(),
                value: None,
            });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    if trace {
        eprintln!(
            "trace: compared and hashed {:?} ({} hashed)",
            t0.elapsed(),
            hashed_count
        );
    }
    Ok(ScanResult {
        changes,
        cache_rows,
        files: files.len(),
        hashed: hashed_count,
        unreadable,
    })
}

/// True if git's blob IDs equal the bytes on disk. Line-ending conversion,
/// clean filters (git-lfs) and other attributes make git store something else,
/// so the index cannot be trusted then and every file is hashed. Errs on the
/// side of hashing: that is only slower.
fn seeding_is_exact(store: &Store, files: &[(String, Metadata)]) -> bool {
    let Some(git) = store.git() else {
        return false;
    };
    if files
        .iter()
        .any(|(rel, _)| rel == ".gitattributes" || rel.ends_with("/.gitattributes"))
        || git.common_dir.join("info").join("attributes").exists()
    {
        return false;
    }
    let mut configs = vec![
        git.common_dir.join("config"),
        PathBuf::from("/etc/gitconfig"),
    ];
    if let Some(home) = dirs::home_dir() {
        configs.push(home.join(".gitconfig"));
        configs.push(home.join(".config").join("git").join("config"));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        configs.push(PathBuf::from(xdg).join("git").join("config"));
    }
    !configs.iter().any(|path| {
        let text = fs::read_to_string(path).unwrap_or_default().to_lowercase();
        // Filter drivers (git-lfs adds one globally) only apply through
        // attributes, checked above. Includes could set anything: be careful.
        text.contains("[include")
            || text.lines().any(|line| {
                let line: String = line
                    .split(['#', ';'])
                    .next()
                    .unwrap_or("")
                    .replace([' ', '\t'], "");
                (line.starts_with("autocrlf=") && !line.ends_with("=false"))
                    || line.starts_with("eol=")
                    || line.starts_with("attributesfile=")
            })
    })
}

/// Reads and hashes files on several threads. Unreadable files are left out.
fn hash_files(
    store: &Store,
    need: &[(&str, &Metadata, u32)],
    write: bool,
) -> Result<HashMap<String, Entry>> {
    let root = store.root.as_path();
    let hash_one = |repo: &gix::Repository,
                    rel: &str,
                    meta: &Metadata,
                    mode: u32|
     -> Option<(String, Entry)> {
        let content = read_recorded(&root.join(rel), meta).ok()?;
        let oid = if write {
            odb::write_blob(repo, &content).ok()?
        } else {
            odb::blob_id(&content).ok()?
        };
        Some((rel.to_string(), Entry { oid, mode }))
    };
    let repo = store.repo()?;
    if need.len() < 64 {
        return Ok(need
            .iter()
            .filter_map(|(rel, meta, mode)| hash_one(repo, rel, meta, *mode))
            .collect());
    }
    let shared = repo.clone().into_sync();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let chunk = need.len().div_ceil(threads);
    let results: Vec<Vec<(String, Entry)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = need
            .chunks(chunk)
            .map(|part| {
                let shared = &shared;
                let hash_one = &hash_one;
                scope.spawn(move || {
                    let repo = shared.to_thread_local();
                    part.iter()
                        .filter_map(|(rel, meta, mode)| hash_one(&repo, rel, meta, *mode))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    Ok(results.into_iter().flatten().collect())
}

/// (mtime_ns, ctime_ns, size as u32, ino as u32) -> (blob, mode) for entries of
/// `.git/index` that git considers clean (older than the index file itself).
type Seed = ((i64, i64, u32, u32), gix::ObjectId, u32);

fn git_index_seeds(store: &Store) -> HashMap<String, Seed> {
    let mut out = HashMap::new();
    let Some(git) = store.git().filter(|g| g.sha1) else {
        return out;
    };
    let path = git.index_file();
    let Ok(index_meta) = fs::metadata(&path) else {
        return out;
    };
    let index_mtime = index_meta.mtime() * 1_000_000_000 + index_meta.mtime_nsec();
    let Ok(file) = gix::index::File::at(&path, gix::hash::Kind::Sha1, true, Default::default())
    else {
        return out;
    };
    use gix::index::entry::{Mode, Stage};
    for entry in file.entries() {
        if entry.stage() != Stage::Unconflicted {
            continue;
        }
        let mode = match entry.mode {
            Mode::FILE => MODE_FILE,
            Mode::FILE_EXECUTABLE => MODE_EXEC,
            Mode::SYMLINK => MODE_LINK,
            _ => continue,
        };
        let st = &entry.stat;
        let mtime = st.mtime.secs as i64 * 1_000_000_000 + st.mtime.nsecs as i64;
        if mtime >= index_mtime {
            continue; // racily clean in git's own terms
        }
        let ctime = st.ctime.secs as i64 * 1_000_000_000 + st.ctime.nsecs as i64;
        let Ok(path) = std::str::from_utf8(entry.path(&file)) else {
            continue;
        };
        out.insert(
            path.to_string(),
            ((mtime, ctime, st.size, st.ino), entry.id, mode),
        );
    }
    out
}

/// Records whatever changed since the last event as one event.
pub fn reconcile(store: &mut Store, lock: &WriteLock, meta: &EventMeta) -> Result<Option<i64>> {
    let skip = open_intent_paths(store)?;
    let result = scan(store, &skip, true)?;
    ledger::record(store, lock, meta, &result.changes, &result.cache_rows)
}

/// Records everything, including files with an edit in flight, before a
/// command overwrites the working tree. Pending intents are dropped: what they
/// guarded is now recorded, and their PostToolUse simply records the result.
/// Returns the files that could not be read, which must not be overwritten.
pub fn reconcile_before_write(store: &mut Store, lock: &WriteLock) -> Result<HashSet<String>> {
    let result = scan(store, &HashSet::new(), true)?;
    let meta = EventMeta::cause("external", "external");
    ledger::record(store, lock, &meta, &result.changes, &result.cache_rows)?;
    store.db.execute("DELETE FROM intents", [])?;
    Ok(result.unreadable.into_iter().collect())
}

/// Paths an edit tool is about to change. Intents older than ten minutes are
/// treated as abandoned (the tool was denied or the session died).
pub fn open_intent_paths(store: &Store) -> Result<HashSet<String>> {
    let cutoff = crate::fsutil::now_ns() - 600 * 1_000_000_000;
    let mut stmt = store
        .db
        .prepare_cached("SELECT path FROM intents WHERE opened_ns > ?1")?;
    let rows = stmt.query_map([cutoff], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Reads specific paths (relative to the root) and returns their changes against
/// the stat cache, without walking. Excluded paths are skipped; a path that no
/// longer exists is a deletion.
pub fn read_paths(
    store: &Store,
    paths: &[String],
    oracle: &mut Oracle,
) -> Result<(Vec<FileChange>, Vec<CacheRow>)> {
    let stamp_ns = touch_stamp(&store.vibe)?;
    let repo = store.repo()?;
    let mut changes = Vec::new();
    let mut rows = Vec::new();
    let mut done = HashSet::new();
    for rel in paths {
        if !done.insert(rel.clone()) || oracle.is_excluded(rel) {
            continue;
        }
        let abs = store.root.join(rel);
        let before = cached(store, rel)?;
        match fs::symlink_metadata(&abs) {
            Ok(meta) if meta.is_file() || meta.file_type().is_symlink() => {
                let content = read_recorded(&abs, &meta)?;
                let entry = Entry {
                    oid: odb::write_blob(repo, &content)?,
                    mode: mode_of(&meta),
                };
                let stat = FileStat::of(&meta, stamp_ns);
                if before.map(|c| c.entry) != Some(entry) {
                    changes.push(FileChange {
                        path: rel.clone(),
                        before: before.map(|c| c.entry),
                        after: Some(entry),
                    });
                }
                rows.push(CacheRow {
                    path: rel.clone(),
                    value: Some((stat, entry)),
                });
            }
            Ok(_) => {}
            Err(_) => {
                if let Some(before) = before {
                    changes.push(FileChange {
                        path: rel.clone(),
                        before: Some(before.entry),
                        after: None,
                    });
                    rows.push(CacheRow {
                        path: rel.clone(),
                        value: None,
                    });
                }
            }
        }
    }
    Ok((changes, rows))
}

pub fn new_oracle(store: &Store) -> Oracle {
    Oracle::new(
        &store.root,
        &store.config,
        store.git().map(|g| g.exclude_file()),
    )
}
