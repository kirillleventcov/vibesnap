//! Thin layer over gitoxide: blobs, trees, commits and refs in `.vibe`.

use crate::error::{Result, git_err, msg};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
use std::collections::BTreeMap;

pub const LEDGER_REF: &str = "refs/vibesnap/ledger";
pub const LABELS_PREFIX: &str = "refs/vibesnap/labels/";

/// A file in a tree: its blob and git mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub oid: ObjectId,
    pub mode: u32,
}

pub const MODE_FILE: u32 = 0o100644;
pub const MODE_EXEC: u32 = 0o100755;
pub const MODE_LINK: u32 = 0o120000;

pub fn kind_of(mode: u32) -> EntryKind {
    match mode {
        MODE_EXEC => EntryKind::BlobExecutable,
        MODE_LINK => EntryKind::Link,
        _ => EntryKind::Blob,
    }
}

pub fn empty_tree() -> ObjectId {
    ObjectId::empty_tree(gix::hash::Kind::Sha1)
}

pub fn parse_oid(hex: &str) -> Result<ObjectId> {
    ObjectId::from_hex(hex.as_bytes()).map_err(|e| msg(format!("bad object id {hex}: {e}")))
}

pub fn blob_id(data: &[u8]) -> Result<ObjectId> {
    gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, data).map_err(git_err)
}

/// Stores a blob unless the store (or `.git` through alternates) already has it.
pub fn write_blob(repo: &gix::Repository, data: &[u8]) -> Result<ObjectId> {
    Ok(repo.write_blob(data).map_err(git_err)?.detach())
}

pub fn read_blob(repo: &gix::Repository, oid: ObjectId) -> Result<Vec<u8>> {
    let mut blob = repo
        .find_blob(oid)
        .map_err(|e| msg(format!("missing object {oid}: {e}")))?;
    Ok(blob.take_data())
}

pub fn has_object(repo: &gix::Repository, oid: ObjectId) -> bool {
    repo.has_object(oid)
}

pub fn read_ref(repo: &gix::Repository, name: &str) -> Result<Option<ObjectId>> {
    match repo.try_find_reference(name).map_err(git_err)? {
        Some(r) => Ok(r.target().try_id().map(|id| id.to_owned())),
        None => Ok(None),
    }
}

fn signature(time_secs: i64) -> gix::actor::Signature {
    gix::actor::Signature {
        name: "vibesnap".into(),
        email: "vibesnap@localhost".into(),
        time: gix::date::Time::new(time_secs, 0),
    }
}

/// Moves `name` to `new` only if it still points at `expected` (None: must not exist).
pub fn update_ref(
    repo: &gix::Repository,
    name: &str,
    new: ObjectId,
    expected: Option<ObjectId>,
) -> Result<()> {
    let expected = match expected {
        Some(old) => PreviousValue::MustExistAndMatch(gix::refs::Target::Object(old)),
        None => PreviousValue::MustNotExist,
    };
    set_ref(repo, name, new, expected)
}

pub fn force_ref(repo: &gix::Repository, name: &str, new: ObjectId) -> Result<()> {
    set_ref(repo, name, new, PreviousValue::Any)
}

fn set_ref(
    repo: &gix::Repository,
    name: &str,
    new: ObjectId,
    expected: PreviousValue,
) -> Result<()> {
    let sig = signature(chrono::Utc::now().timestamp());
    let edit = RefEdit {
        change: Change::Update {
            log: LogChange {
                mode: RefLog::AndReference,
                force_create_reflog: false,
                message: "vibesnap".into(),
            },
            expected,
            new: gix::refs::Target::Object(new),
        },
        name: name.try_into().map_err(git_err)?,
        deref: false,
    };
    let mut time_buf = Default::default();
    repo.edit_references_as([edit], Some(sig.to_ref(&mut time_buf)))
        .map_err(git_err)?;
    Ok(())
}

pub fn delete_ref(repo: &gix::Repository, name: &str) -> Result<()> {
    let edit = RefEdit {
        change: Change::Delete {
            expected: PreviousValue::Any,
            log: RefLog::AndReference,
        },
        name: name.try_into().map_err(git_err)?,
        deref: false,
    };
    repo.edit_references_as([edit], None).map_err(git_err)?;
    Ok(())
}

/// All refs under `prefix`, as (name without prefix, target).
pub fn list_refs(repo: &gix::Repository, prefix: &str) -> Result<Vec<(String, ObjectId)>> {
    let platform = repo.references().map_err(git_err)?;
    let mut out = Vec::new();
    for r in platform.prefixed(prefix).map_err(git_err)? {
        let r = r.map_err(|e| msg(e.to_string()))?;
        let name = r.name().as_bstr().to_string();
        if let Some(id) = r.target().try_id() {
            out.push((name.trim_start_matches(prefix).to_string(), id.to_owned()));
        }
    }
    Ok(out)
}

pub struct CommitInfo {
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub message: String,
}

pub fn read_commit(repo: &gix::Repository, oid: ObjectId) -> Result<CommitInfo> {
    let commit = repo.find_commit(oid).map_err(git_err)?;
    let decoded = commit.decode().map_err(git_err)?;
    Ok(CommitInfo {
        tree: decoded.tree(),
        parents: decoded.parents().collect(),
        message: decoded.message.to_string(),
    })
}

pub fn write_commit(
    repo: &gix::Repository,
    tree: ObjectId,
    parents: &[ObjectId],
    message: &str,
    time_secs: i64,
) -> Result<ObjectId> {
    let sig = signature(time_secs);
    let commit = gix::objs::Commit {
        tree,
        parents: parents.iter().copied().collect(),
        author: sig.clone(),
        committer: sig,
        encoding: None,
        message: message.into(),
        extra_headers: vec![],
    };
    Ok(repo.write_object(&commit).map_err(git_err)?.detach())
}

pub enum TreeEdit {
    Upsert(String, Entry),
    Remove(String),
}

/// Applies `edits` to `base` and writes only the trees on the changed paths.
pub fn edit_tree(repo: &gix::Repository, base: ObjectId, edits: &[TreeEdit]) -> Result<ObjectId> {
    let mut editor = repo.edit_tree(base).map_err(git_err)?;
    for edit in edits {
        if let TreeEdit::Remove(path) = edit {
            editor.remove(path.as_str()).map_err(git_err)?;
        }
    }
    for edit in edits {
        if let TreeEdit::Upsert(path, entry) = edit {
            editor
                .upsert(path.as_str(), kind_of(entry.mode), entry.oid)
                .map_err(git_err)?;
        }
    }
    let tree = editor.write().map_err(git_err)?.detach();
    let paths: Vec<&str> = edits
        .iter()
        .map(|e| match e {
            TreeEdit::Upsert(p, _) | TreeEdit::Remove(p) => p.as_str(),
        })
        .collect();
    keep_trees_local(repo, tree, &paths)?;
    Ok(tree)
}

/// Makes sure the trees on the edited paths are stored in `.vibe` itself.
/// Writing skips objects the store can already see, and an identical tree in
/// `.git` would otherwise be borrowed; if git later pruned it, the ledger tip
/// would become unreadable and no event could be recorded.
fn keep_trees_local(repo: &gix::Repository, tree: ObjectId, paths: &[&str]) -> Result<()> {
    if !repo.git_dir().join("objects/info/alternates").exists() {
        return Ok(());
    }
    let own = gix::odb::loose::Store::at(repo.git_dir().join("objects"), gix::hash::Kind::Sha1);
    let mut dirs: std::collections::HashSet<&str> = std::collections::HashSet::new();
    dirs.insert("");
    for path in paths {
        let mut end = 0;
        while let Some(i) = path[end..].find('/') {
            end += i;
            dirs.insert(&path[..end]);
            end += 1;
        }
    }
    let mut borrowed = Vec::new();
    find_borrowed_trees(repo, &own, tree, "", &dirs, &mut borrowed)?;
    // Many trees (a first snapshot) go into one pack; git does that far faster
    // than writing each as a loose object. A few are written loose.
    if borrowed.len() > 64 && pack_objects(repo.git_dir(), &borrowed) {
        return Ok(());
    }
    use gix::objs::Write;
    for id in borrowed {
        let object = repo.find_object(id).map_err(git_err)?;
        own.write_buf(object.kind, &object.data).map_err(git_err)?;
    }
    Ok(())
}

/// Collects `tree` and its subtrees on edited paths that are not stored in
/// `.vibe` itself. Subtrees off the edited paths are unchanged from the
/// previous tip, which was checked when it was written.
fn find_borrowed_trees(
    repo: &gix::Repository,
    own: &gix::odb::loose::Store,
    tree: ObjectId,
    prefix: &str,
    dirs: &std::collections::HashSet<&str>,
    out: &mut Vec<ObjectId>,
) -> Result<()> {
    if tree == empty_tree() {
        return Ok(());
    }
    if !own.contains(&tree) {
        out.push(tree);
    }
    for (name, mode, oid) in tree_entries(repo, tree)? {
        if !is_tree_mode(mode) {
            continue;
        }
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if dirs.contains(path.as_str()) {
            find_borrowed_trees(repo, own, oid, &path, dirs, out)?;
        }
    }
    Ok(())
}

/// Packs `ids` into the store with `git pack-objects`, reading them through
/// alternates. False if git is unavailable or fails.
fn pack_objects(git_dir: &std::path::Path, ids: &[ObjectId]) -> bool {
    use std::io::Write as _;
    let child = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["pack-objects", "-q"])
        .arg(git_dir.join("objects/pack/pack"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let list: String = ids.iter().map(|id| format!("{id}\n")).collect();
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(list.as_bytes()).is_ok());
    child.wait().is_ok_and(|status| status.success()) && written
}

fn tree_entries(repo: &gix::Repository, tree: ObjectId) -> Result<Vec<(String, u32, ObjectId)>> {
    if tree == empty_tree() {
        return Ok(Vec::new());
    }
    let tree = repo.find_tree(tree).map_err(git_err)?;
    let decoded = tree.decode().map_err(git_err)?;
    Ok(decoded
        .entries
        .iter()
        .map(|e| {
            (
                e.filename.to_string(),
                e.mode.value() as u32,
                e.oid.to_owned(),
            )
        })
        .collect())
}

fn is_tree_mode(mode: u32) -> bool {
    mode & 0o170000 == 0o040000
}

fn is_file_mode(mode: u32) -> bool {
    matches!(mode, MODE_FILE | MODE_EXEC | MODE_LINK) || mode == 0o100664
}

/// Every file in `tree`, by path. Submodule entries are skipped.
pub fn flatten(repo: &gix::Repository, tree: ObjectId) -> Result<BTreeMap<String, Entry>> {
    let mut out = BTreeMap::new();
    flatten_into(repo, tree, "", &[], &mut out)?;
    Ok(out)
}

/// Like `flatten`, but only descends into directories on the way to `scope`.
pub fn flatten_scoped(
    repo: &gix::Repository,
    tree: ObjectId,
    scope: &[String],
) -> Result<BTreeMap<String, Entry>> {
    let mut out = BTreeMap::new();
    flatten_into(repo, tree, "", scope, &mut out)?;
    Ok(out)
}

fn scope_allows_dir(dir: &str, scope: &[String]) -> bool {
    scope.is_empty()
        || scope.iter().any(|p| {
            p.is_empty()
                || crate::paths::in_scope(dir, std::slice::from_ref(p))
                || p.starts_with(&format!("{dir}/"))
        })
}

fn flatten_into(
    repo: &gix::Repository,
    tree: ObjectId,
    prefix: &str,
    scope: &[String],
    out: &mut BTreeMap<String, Entry>,
) -> Result<()> {
    for (name, mode, oid) in tree_entries(repo, tree)? {
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if is_tree_mode(mode) {
            if scope_allows_dir(&path, scope) {
                flatten_into(repo, oid, &path, scope, out)?;
            }
        } else if is_file_mode(mode) && crate::paths::in_scope(&path, scope) {
            out.insert(
                path,
                Entry {
                    oid,
                    mode: normalize_mode(mode),
                },
            );
        }
    }
    Ok(())
}

fn normalize_mode(mode: u32) -> u32 {
    if mode == 0o100664 { MODE_FILE } else { mode }
}

#[derive(Debug, Clone)]
pub struct TreeChange {
    pub path: String,
    pub before: Option<Entry>,
    pub after: Option<Entry>,
}

/// The files that differ between two trees. Subtrees with equal IDs are skipped
/// without being read, so the cost follows the size of the change.
pub fn diff_trees(repo: &gix::Repository, a: ObjectId, b: ObjectId) -> Result<Vec<TreeChange>> {
    let mut out = Vec::new();
    diff_into(repo, Some(a), Some(b), "", &mut out)?;
    Ok(out)
}

fn diff_into(
    repo: &gix::Repository,
    a: Option<ObjectId>,
    b: Option<ObjectId>,
    prefix: &str,
    out: &mut Vec<TreeChange>,
) -> Result<()> {
    if a == b {
        return Ok(());
    }
    let left: BTreeMap<String, (u32, ObjectId)> = match a {
        Some(t) => tree_entries(repo, t)?
            .into_iter()
            .map(|(n, m, o)| (n, (m, o)))
            .collect(),
        None => BTreeMap::new(),
    };
    let right: BTreeMap<String, (u32, ObjectId)> = match b {
        Some(t) => tree_entries(repo, t)?
            .into_iter()
            .map(|(n, m, o)| (n, (m, o)))
            .collect(),
        None => BTreeMap::new(),
    };
    let names: std::collections::BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    for name in names {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let l = left.get(name);
        let r = right.get(name);
        if l == r {
            continue;
        }
        let l_tree = l.filter(|(m, _)| is_tree_mode(*m)).map(|(_, o)| *o);
        let r_tree = r.filter(|(m, _)| is_tree_mode(*m)).map(|(_, o)| *o);
        let l_file = l.filter(|(m, _)| is_file_mode(*m)).map(|(m, o)| Entry {
            oid: *o,
            mode: normalize_mode(*m),
        });
        let r_file = r.filter(|(m, _)| is_file_mode(*m)).map(|(m, o)| Entry {
            oid: *o,
            mode: normalize_mode(*m),
        });
        if l_tree.is_some() || r_tree.is_some() {
            diff_into(repo, l_tree, r_tree, &path, out)?;
        }
        if l_file != r_file && (l_file.is_some() || r_file.is_some()) {
            out.push(TreeChange {
                path,
                before: l_file,
                after: r_file,
            });
        }
    }
    Ok(())
}

/// The entry at `path` in `tree`, if it is a file.
pub fn entry_at(repo: &gix::Repository, tree: ObjectId, path: &str) -> Result<Option<Entry>> {
    if tree == empty_tree() {
        return Ok(None);
    }
    let tree = repo.find_tree(tree).map_err(git_err)?;
    let found = tree.lookup_entry_by_path(path).map_err(git_err)?;
    Ok(found.and_then(|e| {
        let mode = e.mode().value() as u32;
        is_file_mode(mode).then(|| Entry {
            oid: e.object_id(),
            mode: normalize_mode(mode),
        })
    }))
}

/// Parses `Key: value` trailer lines from the end of a commit message.
pub fn parse_trailers(message: &str) -> BTreeMap<String, String> {
    message
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(": ")?;
            key.starts_with("Vibesnap-")
                .then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}
