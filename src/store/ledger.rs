//! Events: one commit per change on `refs/vibesnap/ledger`, mirrored in SQLite.

use super::Store;
use super::index::{self, Event};
use super::lock::WriteLock;
use super::odb::{self, Entry, LEDGER_REF, TreeEdit};
use crate::error::{Result, msg};
use crate::fsutil::now_ns;
use crate::text::line_counts;
use gix::ObjectId;
use rusqlite::params;

/// Who and what caused an event.
#[derive(Debug, Clone, Default)]
pub struct EventMeta {
    /// edit, write, notebook, bash, external, snap, restore, undo, baseline, import
    pub cause: String,
    /// agent, user or external
    pub actor: String,
    pub session_id: Option<String>,
    pub turn: Option<i64>,
    pub prompt_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub tool: Option<String>,
    pub tool_use_id: Option<String>,
    pub note: Option<String>,
    pub ambiguous: Option<String>,
}

impl EventMeta {
    pub fn cause(cause: &str, actor: &str) -> EventMeta {
        EventMeta {
            cause: cause.into(),
            actor: actor.into(),
            ..Default::default()
        }
    }
}

/// One path's change, as the caller observed it.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub before: Option<Entry>,
    pub after: Option<Entry>,
}

/// A stat-cache row to write together with the event (None: delete the row).
#[derive(Debug, Clone)]
pub struct CacheRow {
    pub path: String,
    pub value: Option<(crate::wc::scan::FileStat, Entry)>,
}

/// The ledger's tip commit and its tree.
pub fn tip(store: &Store) -> Result<Option<(ObjectId, ObjectId)>> {
    let repo = store.repo()?;
    match odb::read_ref(repo, LEDGER_REF)? {
        Some(commit) => Ok(Some((commit, odb::read_commit(repo, commit)?.tree))),
        None => Ok(None),
    }
}

pub fn tip_tree(store: &Store) -> Result<ObjectId> {
    Ok(tip(store)?.map(|(_, t)| t).unwrap_or_else(odb::empty_tree))
}

/// Records `changes` as one event, unless they amount to nothing. Writes objects,
/// then moves the ledger ref (compare-and-swap), then commits the index rows and
/// `cache` rows in one SQLite transaction. Returns the event's sequence number.
pub fn record(
    store: &mut Store,
    lock: &WriteLock,
    meta: &EventMeta,
    changes: &[FileChange],
    cache: &[CacheRow],
) -> Result<Option<i64>> {
    catch_up(store, lock)?;
    let changes: Vec<&FileChange> = changes.iter().filter(|c| c.before != c.after).collect();
    let tip = tip(store)?;
    let tip_tree = tip.map(|(_, t)| t).unwrap_or_else(odb::empty_tree);

    let new_tree = if changes.is_empty() {
        tip_tree
    } else {
        let edits: Vec<TreeEdit> = changes
            .iter()
            .map(|c| match c.after {
                Some(entry) => TreeEdit::Upsert(c.path.clone(), entry),
                None => TreeEdit::Remove(c.path.clone()),
            })
            .collect();
        odb::edit_tree(store.repo()?, tip_tree, &edits)?
    };
    if new_tree == tip_tree && tip.is_some() {
        write_cache(&store.db, cache)?;
        return Ok(None);
    }

    let seq = index::next_seq(&store.db)?;
    let ts_ns = now_ns();
    let message = commit_message(seq, ts_ns, meta, &changes);
    let repo = store.repo()?;
    let parents: Vec<ObjectId> = tip.map(|(c, _)| vec![c]).unwrap_or_default();
    let commit = odb::write_commit(repo, new_tree, &parents, &message, ts_ns / 1_000_000_000)?;
    odb::update_ref(repo, LEDGER_REF, commit, tip.map(|(c, _)| c))?;

    let counts: Vec<Option<(i64, i64)>> = if changes.len() <= 200 {
        changes.iter().map(|c| counts_for(repo, c)).collect()
    } else {
        vec![None; changes.len()]
    };

    let tx = store.db.transaction()?;
    insert_event(
        &tx,
        &Event {
            seq,
            commit_oid: commit.to_string(),
            ts_ns,
            before_tree: tip.map(|_| tip_tree.to_string()),
            after_tree: new_tree.to_string(),
            cause: meta.cause.clone(),
            actor: meta.actor.clone(),
            session_id: meta.session_id.clone(),
            turn: meta.turn,
            prompt_id: meta.prompt_id.clone(),
            agent_id: meta.agent_id.clone(),
            agent_type: meta.agent_type.clone(),
            tool: meta.tool.clone(),
            tool_use_id: meta.tool_use_id.clone(),
            lane: "main".into(),
            note: meta.note.clone(),
            ambiguous: meta.ambiguous.clone(),
        },
    )?;
    for (change, count) in changes.iter().zip(counts) {
        insert_path(&tx, seq, change, count)?;
    }
    write_cache(&tx, cache)?;
    tx.commit()?;
    Ok(Some(seq))
}

fn counts_for(repo: &gix::Repository, change: &FileChange) -> Option<(i64, i64)> {
    const LIMIT: u64 = 1024 * 1024;
    let read = |e: Option<Entry>| -> Option<Vec<u8>> {
        match e {
            None => Some(Vec::new()),
            Some(e) if e.mode == odb::MODE_LINK => None,
            Some(e) => {
                let header = repo.find_header(e.oid).ok()?;
                if header.size() > LIMIT {
                    return None;
                }
                odb::read_blob(repo, e.oid).ok()
            }
        }
    };
    line_counts(&read(change.before)?, &read(change.after)?)
}

fn commit_message(seq: i64, ts_ns: i64, meta: &EventMeta, changes: &[&FileChange]) -> String {
    let subject = match changes.len() {
        0 => meta.cause.clone(),
        1 => format!("{} {}", meta.cause, changes[0].path),
        n => format!("{} {} and {} more", meta.cause, changes[0].path, n - 1),
    };
    let mut trailers = vec![
        ("Seq", Some(seq.to_string())),
        ("Ts", Some(ts_ns.to_string())),
        ("Cause", Some(meta.cause.clone())),
        ("Actor", Some(meta.actor.clone())),
        ("Session", meta.session_id.clone()),
        ("Turn", meta.turn.map(|t| t.to_string())),
        ("Prompt", meta.prompt_id.clone()),
        ("Agent-Id", meta.agent_id.clone()),
        ("Agent-Type", meta.agent_type.clone()),
        ("Tool", meta.tool.clone()),
        ("Tool-Use", meta.tool_use_id.clone()),
        ("Ambiguous", meta.ambiguous.clone()),
    ];
    trailers.push(("Note", meta.note.as_ref().map(|n| n.replace('\n', "\\n"))));
    let mut message = format!("{subject}\n\n");
    for (key, value) in trailers {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            message.push_str(&format!("Vibesnap-{key}: {}\n", value.replace('\n', " ")));
        }
    }
    message
}

fn insert_event(conn: &rusqlite::Connection, e: &Event) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO events ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            index::EVENT_COLUMNS
        ),
        params![
            e.seq,
            e.commit_oid,
            e.ts_ns,
            e.before_tree,
            e.after_tree,
            e.cause,
            e.actor,
            e.session_id,
            e.turn,
            e.prompt_id,
            e.agent_id,
            e.agent_type,
            e.tool,
            e.tool_use_id,
            e.lane,
            e.note,
            e.ambiguous
        ],
    )?;
    Ok(())
}

fn insert_path(
    conn: &rusqlite::Connection,
    seq: i64,
    c: &FileChange,
    count: Option<(i64, i64)>,
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT OR REPLACE INTO event_paths (seq, path, before_oid, after_oid, before_mode, after_mode, added, removed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    stmt.execute(params![
        seq,
        c.path,
        c.before.map(|e| e.oid.to_string()),
        c.after.map(|e| e.oid.to_string()),
        c.before.map(|e| e.mode),
        c.after.map(|e| e.mode),
        count.map(|c| c.0),
        count.map(|c| c.1)
    ])?;
    Ok(())
}

pub fn write_cache(conn: &rusqlite::Connection, rows: &[CacheRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let mut upsert = conn.prepare_cached(
        "INSERT INTO stat_cache (lane, path, mtime_ns, ctime_ns, size, ino, mode, oid, racy)
         VALUES ('main', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(lane, path) DO UPDATE SET mtime_ns = excluded.mtime_ns, ctime_ns = excluded.ctime_ns,
           size = excluded.size, ino = excluded.ino, mode = excluded.mode, oid = excluded.oid, racy = excluded.racy",
    )?;
    let mut delete =
        conn.prepare_cached("DELETE FROM stat_cache WHERE lane = 'main' AND path = ?1")?;
    for row in rows {
        match &row.value {
            Some((stat, entry)) => {
                upsert.execute(params![
                    row.path,
                    stat.mtime_ns,
                    stat.ctime_ns,
                    stat.size,
                    stat.ino,
                    entry.mode,
                    entry.oid.to_string(),
                    stat.racy
                ])?;
            }
            None => {
                delete.execute([&row.path])?;
            }
        }
    }
    Ok(())
}

/// Brings the index up to the ledger tip if a crash happened between moving the
/// ref and committing the index rows.
pub fn catch_up(store: &mut Store, _lock: &WriteLock) -> Result<()> {
    let Some((tip_commit, _)) = tip(store)? else {
        return Ok(());
    };
    let indexed = index::last_event(&store.db)?;
    if indexed.as_ref().map(|e| e.commit_oid.as_str()) == Some(tip_commit.to_string().as_str()) {
        return Ok(());
    }
    let repo = store.repo()?;
    let mut missing = Vec::new();
    let mut cursor = Some(tip_commit);
    while let Some(commit) = cursor {
        if indexed
            .as_ref()
            .is_some_and(|e| odb::parse_oid(&e.commit_oid).ok() == Some(commit))
        {
            break;
        }
        let info = odb::read_commit(repo, commit)?;
        cursor = info.parents.first().copied();
        missing.push((commit, info));
        if cursor.is_none() && indexed.is_some() {
            // The indexed tip is not an ancestor: the ledger was rewritten elsewhere.
            return reindex(store);
        }
    }
    missing.reverse();
    let rows: Vec<(Event, Vec<FileChange>)> = missing
        .into_iter()
        .map(|(commit, info)| event_from_commit(repo, commit, &info))
        .collect::<Result<_>>()?;
    let tx = store.db.transaction()?;
    for (event, changes) in rows {
        insert_event(&tx, &event)?;
        for c in &changes {
            insert_path(&tx, event.seq, c, None)?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Folds the WAL back into the database and truncates it, after large writes.
pub fn checkpoint(db: &rusqlite::Connection) {
    let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
}

fn event_from_commit(
    repo: &gix::Repository,
    commit: ObjectId,
    info: &odb::CommitInfo,
) -> Result<(Event, Vec<FileChange>)> {
    let t = odb::parse_trailers(&info.message);
    let get = |k: &str| t.get(&format!("Vibesnap-{k}")).cloned();
    let parent_tree = match info.parents.first() {
        Some(p) => Some(odb::read_commit(repo, *p)?.tree),
        None => None,
    };
    let seq: i64 = get("Seq")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| msg(format!("ledger commit {commit} has no sequence number")))?;
    let changes = odb::diff_trees(repo, parent_tree.unwrap_or_else(odb::empty_tree), info.tree)?
        .into_iter()
        .map(|c| FileChange {
            path: c.path,
            before: c.before,
            after: c.after,
        })
        .collect();
    let event = Event {
        seq,
        commit_oid: commit.to_string(),
        ts_ns: get("Ts").and_then(|s| s.parse().ok()).unwrap_or_default(),
        before_tree: parent_tree.map(|t| t.to_string()),
        after_tree: info.tree.to_string(),
        cause: get("Cause").unwrap_or_else(|| "unknown".into()),
        actor: get("Actor").unwrap_or_else(|| "unknown".into()),
        session_id: get("Session"),
        turn: get("Turn").and_then(|s| s.parse().ok()),
        prompt_id: get("Prompt"),
        agent_id: get("Agent-Id"),
        agent_type: get("Agent-Type"),
        tool: get("Tool"),
        tool_use_id: get("Tool-Use"),
        lane: "main".into(),
        note: get("Note").map(|n| n.replace("\\n", "\n")),
        ambiguous: get("Ambiguous"),
    };
    Ok((event, changes))
}

/// Rebuilds events, paths and turn spans from the ledger, and drops stat-cache
/// rows that disagree with the ledger tip.
pub fn reindex(store: &mut Store) -> Result<()> {
    let repo = store.repo()?;
    let mut commits = Vec::new();
    let mut cursor = odb::read_ref(repo, LEDGER_REF)?;
    while let Some(commit) = cursor {
        let info = odb::read_commit(repo, commit)?;
        cursor = info.parents.first().copied();
        commits.push((commit, info));
    }
    commits.reverse();
    let rows: Vec<(Event, Vec<FileChange>)> = commits
        .iter()
        .map(|(c, info)| event_from_commit(repo, *c, info))
        .collect::<Result<_>>()?;
    let tip_files = match commits.last() {
        Some((_, info)) => odb::flatten(repo, info.tree)?,
        None => Default::default(),
    };

    let counts: Vec<Vec<Option<(i64, i64)>>> = rows
        .iter()
        .map(|(_, changes)| {
            changes
                .iter()
                .map(|c| {
                    if changes.len() <= 200 {
                        counts_for(repo, c)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect();

    let tx = store.db.transaction()?;
    tx.execute_batch(
        "DELETE FROM events; DELETE FROM event_paths; DELETE FROM spans WHERE kind = 'turn';",
    )?;
    for ((event, changes), counts) in rows.iter().zip(counts) {
        insert_event(&tx, event)?;
        for (c, count) in changes.iter().zip(counts) {
            insert_path(&tx, event.seq, c, count)?;
        }
    }
    // Turn spans: the events of each turn number, starting just before its first event.
    tx.execute(
        "INSERT OR REPLACE INTO spans (id, kind, session_id, number, start_seq, end_seq, start_ns, end_ns)
         SELECT 'turn-' || turn, 'turn', MAX(session_id), turn, MIN(seq) - 1, MAX(seq), MIN(ts_ns), MAX(ts_ns)
         FROM events WHERE turn IS NOT NULL GROUP BY turn",
        [],
    )?;
    let cached: Vec<(String, String, u32)> = {
        let mut stmt = tx.prepare("SELECT path, oid, mode FROM stat_cache WHERE lane = 'main'")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (path, oid, mode) in cached {
        let keep = tip_files
            .get(&path)
            .is_some_and(|e| odb::parse_oid(&oid).ok() == Some(e.oid) && e.mode == mode);
        if !keep {
            tx.execute(
                "DELETE FROM stat_cache WHERE lane = 'main' AND path = ?1",
                [&path],
            )?;
        }
    }
    // Paths in the ledger with no cache row get a row that never matches, so the
    // next scan hashes them instead of reporting them deleted.
    for (path, entry) in &tip_files {
        tx.execute(
            "INSERT OR IGNORE INTO stat_cache (lane, path, mtime_ns, ctime_ns, size, ino, mode, oid, racy)
             VALUES ('main', ?1, -1, -1, -1, -1, ?2, ?3, 1)",
            params![path, entry.mode, entry.oid.to_string()],
        )?;
    }
    tx.commit()?;
    checkpoint(&store.db);
    Ok(())
}
