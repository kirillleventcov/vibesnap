//! Commands that write: init, snap, undo, restore, label, reset. Plus status.

use super::out::{Out, clock_of};
use crate::error::{AppError, Result, git_err, msg};
use crate::paths::{in_scope, resolve_user_path};
use crate::refs::{self, Target};
use crate::store::index::{self, Event};
use crate::store::ledger::{self, EventMeta};
use crate::store::lock::{self, WriteLock};
use crate::store::odb::{self, Entry, LABELS_PREFIX, MODE_LINK};
use crate::store::{STORE_DIR, Store};
use crate::text::is_binary;
use crate::wc::apply::{Applied, apply, blocked, write_entry};
use crate::wc::scan::{
    self, load_cache, new_oracle, open_intent_paths, read_paths, reconcile, reconcile_before_write,
};
use colored::Colorize;
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Opens the store, takes the writer lock and applies queued hook events.
pub fn begin_write() -> Result<(Store, WriteLock)> {
    let mut store = Store::find()?;
    let lock = lock::lock(&store.vibe)?;
    crate::hook::drain_inbox(&mut store, &lock)?;
    Ok((store, lock))
}

pub fn resolve_scope(store: &Store, paths: &[PathBuf]) -> Result<Vec<String>> {
    let scope: Vec<String> = paths
        .iter()
        .map(|p| resolve_user_path(&store.root, p))
        .collect::<Result<_>>()?;
    Ok(if scope.iter().any(|p| p.is_empty()) {
        Vec::new()
    } else {
        scope
    })
}

/// The recorded state of the working tree (it mirrors the ledger tip).
pub fn recorded_map(store: &Store) -> Result<BTreeMap<String, Entry>> {
    Ok(load_cache(store)?
        .into_iter()
        .map(|(p, c)| (p, c.entry))
        .collect())
}

pub fn init(out: &Out, path: &Path) -> Result<()> {
    let existed = path.join(STORE_DIR).exists();
    let store = Store::init(path)?;
    let last = index::last_event(&store.db)?;
    let files = load_cache(&store)?.len();
    let borrowing = store.vibe.join("objects/info/alternates").exists();
    if out.json {
        out.emit(json!({
            "root": store.root, "created": !existed, "event": last.as_ref().map(|e| e.seq),
            "files": files, "git": store.git().is_some(), "borrowing_from_git": borrowing,
        }));
    } else if existed {
        println!("Already initialised at {}", store.root.display());
    } else {
        println!(
            "Initialised vibesnap in {} and recorded {} files as e:{}{}",
            store.vibe.display(),
            files,
            last.map(|e| e.seq).unwrap_or(0),
            if borrowing {
                " (sharing objects with .git)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

pub fn snap(out: &Out, message: Option<String>, label: Option<String>) -> Result<()> {
    let (mut store, lock) = begin_write()?;
    let mut meta = EventMeta::cause("snap", "user");
    meta.note = message;
    let seq = reconcile(&mut store, &lock, &meta)?;
    let at = index::last_seq(&store.db)?;
    if let Some(name) = &label {
        let state = refs::state_at(&store, at, format!("e:{at}"))?;
        create_label(&store, name, &state, true)?;
    }
    if out.json {
        out.emit(json!({"created": seq.is_some(), "event": at, "label": label}));
    } else {
        match seq {
            Some(seq) => println!("Recorded e:{seq}"),
            None => println!("Nothing changed since e:{at}"),
        }
        if let Some(name) = label {
            println!("Labelled e:{at} as {name}");
        }
    }
    Ok(())
}

fn status_letter(before: Option<Entry>, after: Option<Entry>) -> &'static str {
    match (before, after) {
        (None, _) => "added",
        (_, None) => "deleted",
        _ => "modified",
    }
}

pub fn status(out: &Out) -> Result<()> {
    let store = Store::find()?;
    let skip = open_intent_paths(&store)?;
    let result = scan::scan(&store, &skip, false)?;
    let last = index::last_event(&store.db)?;
    let turn = index::spans(&store.db, "turn")?.into_iter().next();
    let errors = recent_hook_errors(&store);
    let inbox = fs::read_dir(store.vibe.join("inbox"))
        .map(|d| d.count())
        .unwrap_or(0);
    if out.json {
        out.emit(json!({
            "last_event": last,
            "turn": turn.as_ref().and_then(|t| t.number),
            "changes": result.changes.iter().map(|c| json!({"path": c.path, "status": status_letter(c.before, c.after)})).collect::<Vec<_>>(),
            "files": result.files,
            "pending_hook_events": inbox,
            "hook_errors": errors,
        }));
        return Ok(());
    }
    match &last {
        Some(e) => println!(
            "Last event e:{} at {} by {} ({}){}",
            e.seq,
            clock_of(e.ts_ns),
            e.who(),
            e.cause,
            e.turn.map(|t| format!(", turn:{t}")).unwrap_or_default()
        ),
        None => println!("No events recorded yet."),
    }
    if result.changes.is_empty() {
        println!(
            "Nothing changed since then ({} files tracked).",
            result.files
        );
    } else {
        println!("Changed since then:");
        for c in &result.changes {
            let letter = match status_letter(c.before, c.after) {
                "added" => "A".green(),
                "deleted" => "D".red(),
                _ => "M".yellow(),
            };
            println!("  {letter} {}", c.path);
        }
    }
    if inbox > 0 {
        println!(
            "{} hook events are queued and will be applied by the next write.",
            inbox
        );
    }
    for e in errors {
        println!("{} {e}", "hook error:".red());
    }
    Ok(())
}

/// The last few hook errors from the past day.
pub fn recent_hook_errors(store: &Store) -> Vec<String> {
    let path = store.hook_log();
    let fresh = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 86400);
    if !fresh {
        return Vec::new();
    }
    let content = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    lines[lines.len().saturating_sub(3)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn restore(out: &Out, spec: &str, dry_run: bool, paths: &[PathBuf]) -> Result<()> {
    let (mut store, lock) = begin_write()?;
    let scope = resolve_scope(&store, paths)?;
    let target = match refs::resolve(&store, spec)? {
        Target::Worktree => {
            return Err(AppError::Usage(
                "Restore needs a recorded state, not @".into(),
            ));
        }
        Target::State(s) => s,
    };
    let protected = reconcile_before_write(&mut store, &lock)?;
    let before_seq = index::last_seq(&store.db)?;
    let current = recorded_map(&store)?;
    let target_map = odb::flatten_scoped(store.repo()?, target.tree, &scope)?;
    for prefix in &scope {
        let known = current
            .keys()
            .chain(target_map.keys())
            .any(|p| in_scope(p, std::slice::from_ref(prefix)));
        if !known {
            return Err(AppError::Usage(format!(
                "{prefix} is neither in {spec} nor in the working tree"
            )));
        }
    }
    let delete = !target.partial;
    let applied = if dry_run {
        plan(&current, &target_map, &scope, delete)
    } else {
        let mut oracle = new_oracle(&store);
        apply(
            &store,
            &mut oracle,
            &protected,
            &current,
            &target_map,
            &scope,
            delete,
        )?
    };
    let mut seq = None;
    if !dry_run {
        let touched: Vec<String> = applied
            .written
            .iter()
            .chain(&applied.deleted)
            .cloned()
            .collect();
        let mut oracle = new_oracle(&store);
        let (changes, rows) = read_paths(&store, &touched, &mut oracle)?;
        let mut meta = EventMeta::cause("restore", "user");
        meta.note = Some(format!("restore {spec}"));
        seq = ledger::record(&mut store, &lock, &meta, &changes, &rows)?;
    }
    if out.json {
        out.emit(json!({
            "target": spec, "dry_run": dry_run, "before": before_seq, "event": seq,
            "written": applied.written, "deleted": applied.deleted, "unchanged": applied.unchanged,
            "skipped": applied.skipped, "deletion_enabled": delete,
        }));
        return Ok(());
    }
    println!(
        "{}{}: {} written, {} deleted, {} unchanged",
        if dry_run {
            "Would restore "
        } else {
            "Restored "
        },
        spec.green(),
        applied.written.len(),
        applied.deleted.len(),
        applied.unchanged
    );
    if !dry_run && before_seq > 0 {
        println!("The previous state is e:{before_seq} (vs restore e:{before_seq} to go back).");
    }
    for s in &applied.skipped {
        eprintln!("{}", format!("Skipped {}: {}", s.path, s.reason).yellow());
    }
    if !delete {
        eprintln!(
            "{}",
            format!("{spec} may not describe the whole tree, so nothing was deleted.").yellow()
        );
    }
    Ok(())
}

fn plan(
    current: &BTreeMap<String, Entry>,
    target: &BTreeMap<String, Entry>,
    scope: &[String],
    delete: bool,
) -> Applied {
    let mut applied = Applied::default();
    for (path, entry) in target.iter().filter(|(p, _)| in_scope(p, scope)) {
        if current.get(path) == Some(entry) {
            applied.unchanged += 1;
        } else {
            applied.written.push(path.clone());
        }
    }
    if delete {
        applied.deleted = current
            .keys()
            .filter(|p| in_scope(p, scope) && !target.contains_key(*p))
            .cloned()
            .collect();
    }
    applied
}

struct Conflict {
    path: String,
    seq: i64,
    later: Vec<i64>,
    reason: String,
}

pub fn undo(
    out: &Out,
    spec: &str,
    include_ambiguous: bool,
    include_external: bool,
    dry_run: bool,
    paths: &[PathBuf],
) -> Result<()> {
    let (mut store, lock) = begin_write()?;
    let scope = resolve_scope(&store, paths)?;
    // Resolve before recording the current state, so @-1 means what the caller saw.
    let selected = refs::resolve_events(&store, spec)?;
    // Edits by the user's editor or other tools are kept unless named directly.
    let include_external = include_external || spec.starts_with("e:");
    let mut left_out = Vec::new();
    let events: Vec<Event> = selected
        .into_iter()
        .filter(|e| {
            let skip = matches!(e.cause.as_str(), "baseline" | "import")
                || (e.ambiguous.is_some() && !include_ambiguous)
                || (e.actor == "external" && !include_external);
            if skip {
                left_out.push(e.seq);
            }
            !skip
        })
        .collect();
    if events.is_empty() {
        let list: Vec<String> = left_out.iter().map(|s| format!("e:{s}")).collect();
        return Err(AppError::Usage(format!(
            "Nothing to undo for {spec}{}",
            if list.is_empty() {
                String::new()
            } else {
                format!(
                    " (left out: {}; see --include-ambiguous, --include-external)",
                    list.join(", ")
                )
            }
        )));
    }
    let protected = reconcile_before_write(&mut store, &lock)?;
    let selected_seqs: HashSet<i64> = events.iter().map(|e| e.seq).collect();
    let last = index::last_seq(&store.db)?;
    let repo = store.repo()?;
    let mut working: BTreeMap<String, Option<Entry>> = BTreeMap::new();
    let current = recorded_map(&store)?;
    let mut conflicts = Vec::new();

    for event in events.iter().rev() {
        for row in index::event_paths(&store.db, event.seq)? {
            if !in_scope(&row.path, &scope) {
                continue;
            }
            let before = entry_of(row.before_oid.as_deref(), row.before_mode)?;
            let after = entry_of(row.after_oid.as_deref(), row.after_mode)?;
            let cur = *working
                .entry(row.path.clone())
                .or_insert_with(|| current.get(&row.path).copied());
            let result = if cur == after {
                Ok(before)
            } else {
                merge_inverse(repo, cur, after, before)
            };
            match result {
                Ok(new) => {
                    working.insert(row.path.clone(), new);
                }
                Err(reason) => {
                    let later = index::path_history(&store.db, &row.path, event.seq, last)?
                        .into_iter()
                        .filter(|r| r.path == row.path && !selected_seqs.contains(&r.seq))
                        .map(|r| r.seq)
                        .collect();
                    conflicts.push(Conflict {
                        path: row.path.clone(),
                        seq: event.seq,
                        later,
                        reason,
                    });
                }
            }
        }
    }

    if !conflicts.is_empty() {
        let mut text = format!("Refusing to undo {spec}: later changes overlap.\n");
        for c in &conflicts {
            let later: Vec<String> = c.later.iter().map(|s| format!("e:{s}")).collect();
            text.push_str(&format!(
                "  {} (from e:{}): {}{}\n",
                c.path,
                c.seq,
                c.reason,
                if later.is_empty() {
                    String::new()
                } else {
                    format!("; changed later by {}", later.join(", "))
                }
            ));
        }
        text.push_str("Undo fewer events, narrow with -- <paths>, or edit by hand.");
        return Err(AppError::Conflict(text));
    }

    let changed: Vec<(String, Option<Entry>)> = working
        .into_iter()
        .filter(|(p, e)| current.get(p).copied() != *e)
        .collect();
    // Decide for every path before writing any: an undo is all or nothing.
    let mut oracle = new_oracle(&store);
    let refused: Vec<String> = changed
        .iter()
        .filter_map(|(p, _)| {
            blocked(&store.root, &mut oracle, &protected, p).map(|why| format!("  {p}: {why}"))
        })
        .collect();
    if !refused.is_empty() {
        return Err(AppError::Conflict(format!(
            "Refusing to undo {spec}: these files cannot be changed safely.\n{}",
            refused.join("\n")
        )));
    }
    let mut seq = None;
    if !dry_run {
        for (path, entry) in &changed {
            let abs = store.root.join(path);
            match entry {
                Some(entry) => {
                    let existing = fs::symlink_metadata(&abs).ok();
                    write_entry(&store, &abs, *entry, existing.as_ref())
                        .map_err(|e| msg(format!("{path}: {e}")))?;
                }
                None => {
                    if fs::remove_file(&abs).is_ok() {
                        crate::fsutil::remove_empty_parents(&store.root, &abs);
                    }
                }
            }
        }
        let touched: Vec<String> = changed.iter().map(|(p, _)| p.clone()).collect();
        let mut oracle = new_oracle(&store);
        let (file_changes, rows) = read_paths(&store, &touched, &mut oracle)?;
        let mut meta = EventMeta::cause("undo", "user");
        meta.note = Some(format!("undo {spec}"));
        seq = ledger::record(&mut store, &lock, &meta, &file_changes, &rows)?;
    }

    let undone: Vec<i64> = events.iter().map(|e| e.seq).collect();
    if out.json {
        out.emit(json!({
            "spec": spec, "dry_run": dry_run, "undone": undone, "left_out": left_out, "event": seq,
            "files": changed.iter().map(|(p, e)| json!({"path": p, "status": if e.is_some() { "restored" } else { "deleted" }})).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if changed.is_empty() {
        println!("Nothing to change: the working tree already lacks those edits.");
    } else {
        println!(
            "{} {} events ({} files){}",
            if dry_run { "Would undo" } else { "Undid" },
            undone.len(),
            changed.len(),
            seq.map(|s| format!(" as e:{s}; vs undo e:{s} reverts this"))
                .unwrap_or_default()
        );
        for (path, entry) in &changed {
            println!("  {} {path}", if entry.is_some() { "M" } else { "D" });
        }
    }
    if !left_out.is_empty() {
        let list: Vec<String> = left_out.iter().map(|s| format!("e:{s}")).collect();
        println!(
            "Left out (baseline, import, external or ambiguous): {}",
            list.join(", ")
        );
    }
    Ok(())
}

fn entry_of(oid: Option<&str>, mode: Option<u32>) -> Result<Option<Entry>> {
    match oid {
        Some(oid) => Ok(Some(Entry {
            oid: odb::parse_oid(oid)?,
            mode: mode.unwrap_or(odb::MODE_FILE),
        })),
        None => Ok(None),
    }
}

/// Applies the inverse of `after -> before` to `current` by a three-way merge
/// with `after` as the base. Only text files can be merged.
fn merge_inverse(
    repo: &gix::Repository,
    current: Option<Entry>,
    after: Option<Entry>,
    before: Option<Entry>,
) -> std::result::Result<Option<Entry>, String> {
    let (Some(cur), Some(after), Some(before)) = (current, after, before) else {
        return Err(match (current, after) {
            (None, _) => "the file was deleted since".into(),
            (_, None) => "the file was re-created since".into(),
            _ => "the file was created by this event and changed since".into(),
        });
    };
    if [cur, after, before].iter().any(|e| e.mode == MODE_LINK) {
        return Err("symlink changed since".into());
    }
    let read = |e: Entry| odb::read_blob(repo, e.oid).map_err(|e| e.to_string());
    let (c, a, b) = (read(cur)?, read(after)?, read(before)?);
    if is_binary(&c) || is_binary(&a) || is_binary(&b) {
        return Err("binary file changed since".into());
    }
    // Merge bytes, not decoded text, so files in other encodings survive.
    match diffy::merge_bytes(&a, &c, &b) {
        Ok(merged) => {
            let oid = odb::write_blob(repo, &merged).map_err(|e| e.to_string())?;
            Ok(Some(Entry {
                oid,
                mode: cur.mode,
            }))
        }
        Err(_) => Err("overlapping lines changed since".into()),
    }
}

pub fn label(
    out: &Out,
    name: Option<String>,
    spec: Option<String>,
    delete: bool,
    force: bool,
) -> Result<()> {
    let Some(name) = name else {
        return list_labels(out);
    };
    if delete {
        let (store, _lock) = begin_write()?;
        let repo = store.repo()?;
        let full = format!("{LABELS_PREFIX}{name}");
        if odb::read_ref(repo, &full)?.is_none() {
            return Err(AppError::Usage(format!("No label {name}")));
        }
        odb::delete_ref(repo, &full)?;
        if out.json {
            out.emit(json!({"deleted": name}));
        } else {
            println!("Deleted label {name}");
        }
        return Ok(());
    }
    let (mut store, lock) = begin_write()?;
    let state = match spec.as_deref().unwrap_or("@") {
        "@" => {
            reconcile_before_write(&mut store, &lock)?;
            let at = index::last_seq(&store.db)?;
            refs::state_at(&store, at, format!("e:{at}"))?
        }
        other => refs::resolve_state(&store, other)?,
    };
    create_label(&store, &name, &state, force)?;
    if out.json {
        out.emit(json!({"label": name, "event": state.seq, "tree": state.tree.to_string()}));
    } else {
        println!(
            "Labelled {} as {name}",
            state.seq.map(|s| format!("e:{s}")).unwrap_or(state.name)
        );
    }
    Ok(())
}

pub fn create_label(store: &Store, name: &str, state: &refs::State, force: bool) -> Result<()> {
    if !refs::is_label_name(name)
        || ["turn", "session"].contains(&name)
        || name.starts_with("turn-")
        || name.contains(':')
    {
        return Err(AppError::Usage(format!(
            "Invalid label name {name}: use letters, digits, '-', '_' and '.', not a ref keyword"
        )));
    }
    let repo = store.repo()?;
    let full = format!("{LABELS_PREFIX}{name}");
    if !force && odb::read_ref(repo, &full)?.is_some() {
        return Err(AppError::Usage(format!(
            "Label {name} exists; pass --force to move it"
        )));
    }
    let mut message = format!("label {name}\n\nVibesnap-Label: {name}\n");
    if let Some(seq) = state.seq {
        message.push_str(&format!("Vibesnap-Seq: {seq}\n"));
    }
    if state.partial {
        message.push_str("Vibesnap-Partial: true\n");
    }
    let commit = odb::write_commit(
        repo,
        state.tree,
        &[],
        &message,
        chrono::Utc::now().timestamp(),
    )?;
    copy_borrowed(store, state.tree)?;
    odb::force_ref(repo, &full, commit)
}

/// Copies every object of `tree` that is only available through `.git` into the
/// store, so a label survives git pruning those objects.
fn copy_borrowed(store: &Store, tree: gix::ObjectId) -> Result<()> {
    if !store.vibe.join("objects/info/alternates").exists() {
        return Ok(());
    }
    let repo = store.repo()?;
    let own = gix::odb::loose::Store::at(store.vibe.join("objects"), gix::hash::Kind::Sha1);
    let mut stack = vec![tree];
    let mut seen = HashSet::new();
    while let Some(oid) = stack.pop() {
        if !seen.insert(oid) || oid == odb::empty_tree() {
            continue;
        }
        let object = repo.find_object(oid).map_err(git_err)?;
        if object.kind == gix::objs::Kind::Tree {
            let tree = object.clone().into_tree();
            for entry in tree.iter() {
                let entry = entry.map_err(git_err)?;
                if !entry.mode().is_commit() {
                    stack.push(entry.object_id());
                }
            }
        }
        if !own.contains(&oid) {
            use gix::objs::Write;
            own.write_buf(object.kind, &object.data).map_err(git_err)?;
        }
    }
    Ok(())
}

fn list_labels(out: &Out) -> Result<()> {
    let store = Store::find()?;
    let repo = store.repo()?;
    let mut labels = Vec::new();
    for (name, commit) in odb::list_refs(repo, LABELS_PREFIX)? {
        let info = odb::read_commit(repo, commit)?;
        let t = odb::parse_trailers(&info.message);
        labels.push(json!({
            "name": name,
            "event": t.get("Vibesnap-Seq").and_then(|s| s.parse::<i64>().ok()),
            "partial": t.get("Vibesnap-Partial").is_some_and(|v| v == "true"),
            "imported": t.contains_key("Vibesnap-Import"),
        }));
    }
    if out.json {
        out.emit(json!({"labels": labels}));
    } else if labels.is_empty() {
        println!("No labels.");
    } else {
        for l in labels {
            println!(
                "{}  {}{}",
                l["name"].as_str().unwrap_or_default(),
                l["event"]
                    .as_i64()
                    .map(|s| format!("e:{s}"))
                    .unwrap_or_else(|| "-".into()),
                if l["partial"].as_bool() == Some(true) {
                    "  (partial)"
                } else {
                    ""
                }
            );
        }
    }
    Ok(())
}

pub fn reset(out: &Out, confirm: bool) -> Result<()> {
    if !confirm {
        return Err(AppError::Usage(
            "Refusing to delete the store and all history without --confirm".into(),
        ));
    }
    let store = Store::find()?;
    let _lock = lock::lock(&store.vibe)?;
    fs::remove_dir_all(&store.vibe)?;
    if out.json {
        out.emit(json!({"removed": store.vibe}));
    } else {
        println!("Deleted {}", store.vibe.display());
    }
    Ok(())
}
