//! Claude Code hook payloads mapped onto events.
//!
//! Rule: reconcile before, attribute after. Before an agent acts, whatever
//! changed since the last event is recorded as `external`; the agent's own
//! event then holds only its own changes.

use super::{drain_inbox, enqueue, log_error};
use crate::error::{Result, msg};
use crate::fsutil::now_ns;
use crate::paths::resolve_from;
use crate::store::Store;
use crate::store::index;
use crate::store::ledger::{self, CacheRow, EventMeta};
use crate::store::lock::{self, WriteLock};
use crate::store::odb::{self, Entry};
use crate::wc::scan::{self, FileStat, mode_of, new_oracle, read_paths, read_recorded, reconcile};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const EDIT_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];
const LOCK_WAIT: Duration = Duration::from_millis(50);

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub fn handle(input: &str) -> Result<Option<String>> {
    let trace = std::env::var_os("VIBESNAP_TRACE").is_some();
    let t0 = std::time::Instant::now();
    let payload: Value =
        serde_json::from_str(input).map_err(|e| msg(format!("bad hook payload: {e}")))?;
    let event = str_of(&payload, "hook_event_name")
        .unwrap_or_default()
        .to_string();
    let cwd = str_of(&payload, "cwd")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| msg("no working directory"))?;
    let root = match Store::discover(&cwd) {
        Some(root) => root,
        None if event == "SessionStart" => match auto_init_root(&cwd) {
            Some(root) => {
                Store::init(&root)?;
                root
            }
            None => return Ok(None),
        },
        None => return Ok(None),
    };
    let mut store = Store::open(&root)?;
    let result = (|| -> Result<Option<String>> {
        if needs_lock(&payload) {
            match lock::try_lock_for(&store.vibe, LOCK_WAIT)? {
                Some(lock) => {
                    drain_inbox(&mut store, &lock)?;
                    apply(&mut store, Some(&lock), &payload, false)
                }
                None => {
                    enqueue(&store, "claude", &payload)?;
                    Ok(None)
                }
            }
        } else {
            apply(&mut store, None, &payload, false)
        }
    })();
    if trace {
        eprintln!("trace: hook done {:?}", t0.elapsed());
    }
    match result {
        Ok(output) => Ok(output),
        Err(e) => {
            log_error(&store, &event, &e);
            Ok(None)
        }
    }
}

fn tool_name(payload: &Value) -> &str {
    str_of(payload, "tool_name").unwrap_or_default()
}

fn needs_lock(payload: &Value) -> bool {
    match str_of(payload, "hook_event_name").unwrap_or_default() {
        "SessionStart" => str_of(payload, "source") != Some("compact"),
        "UserPromptSubmit" => true,
        "PreToolUse" => tool_name(payload) == "Bash",
        "PostToolUse" => tool_name(payload) == "Bash" || EDIT_TOOLS.contains(&tool_name(payload)),
        _ => false,
    }
}

/// The git top level above `cwd`, if a store may be created there.
fn auto_init_root(cwd: &Path) -> Option<PathBuf> {
    let config = crate::config::Config::load();
    if !config.auto_init {
        return None;
    }
    let top = cwd
        .ancestors()
        .find(|d| d.join(".git").exists())?
        .to_path_buf();
    let mut count = 0usize;
    let walker = ignore::WalkBuilder::new(&top)
        .hidden(false)
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|t| t.is_file()) {
            count += 1;
            if count > config.auto_init_max_files {
                return None;
            }
        }
    }
    Some(top)
}

/// Turn number and id for the current turn of a session.
fn current_turn(store: &Store, payload: &Value) -> Result<Option<(String, i64)>> {
    if let Some(prompt) = str_of(payload, "prompt_id")
        && let Some(span) = index::span(&store.db, prompt)?
    {
        return Ok(Some((span.id, span.number.unwrap_or_default())));
    }
    let session = str_of(payload, "session_id");
    Ok(store
        .db
        .query_row(
            "SELECT id, number FROM spans WHERE kind = 'turn' AND (?1 IS NULL OR session_id = ?1)
             ORDER BY start_seq DESC, start_ns DESC LIMIT 1",
            [session],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or_default(),
                ))
            },
        )
        .optional()?)
}

fn meta_for(store: &Store, payload: &Value, cause: &str, actor: &str) -> Result<EventMeta> {
    let agent = actor == "agent";
    Ok(EventMeta {
        cause: cause.into(),
        actor: actor.into(),
        session_id: str_of(payload, "session_id").map(String::from),
        turn: current_turn(store, payload)?.map(|(_, n)| n),
        prompt_id: str_of(payload, "prompt_id").map(String::from),
        agent_id: if agent {
            str_of(payload, "agent_id").map(String::from)
        } else {
            None
        },
        agent_type: if agent {
            str_of(payload, "agent_type").map(String::from)
        } else {
            None
        },
        tool: if agent {
            str_of(payload, "tool_name").map(String::from)
        } else {
            None
        },
        tool_use_id: if agent {
            str_of(payload, "tool_use_id").map(String::from)
        } else {
            None
        },
        note: None,
        ambiguous: None,
    })
}

fn tool_path(store: &Store, payload: &Value) -> Option<String> {
    let input = payload.get("tool_input")?;
    let path = str_of(input, "file_path").or_else(|| str_of(input, "notebook_path"))?;
    let cwd = str_of(payload, "cwd")
        .map(PathBuf::from)
        .unwrap_or_else(|| store.root.clone());
    resolve_from(&store.root, &cwd, Path::new(path))
        .ok()
        .filter(|p| !p.is_empty())
}

fn cause_of(tool: &str) -> &'static str {
    match tool {
        "Write" => "write",
        "NotebookEdit" => "notebook",
        "Bash" => "bash",
        _ => "edit",
    }
}

fn require(lock: Option<&WriteLock>) -> Result<&WriteLock> {
    lock.ok_or_else(|| msg("internal: writer lock not held"))
}

/// Handles one payload. `lock` is held for the events that write to the ledger.
pub fn apply(
    store: &mut Store,
    lock: Option<&WriteLock>,
    payload: &Value,
    queued: bool,
) -> Result<Option<String>> {
    let event = str_of(payload, "hook_event_name").unwrap_or_default();
    let session = str_of(payload, "session_id").map(String::from);
    match event {
        "SessionStart" => {
            if str_of(payload, "source") == Some("compact") {
                let (lines, _) = crate::cli::recap_text(store, false, 40)?;
                if lines.is_empty() {
                    return Ok(None);
                }
                let context = format!(
                    "vibesnap recap of this session's file changes (query more with `vs log`, `vs diff turn-N`):\n{}",
                    lines.join("\n")
                );
                return Ok(Some(
                    json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}})
                        .to_string(),
                ));
            }
            let lock = require(lock)?;
            reconcile(store, lock, &EventMeta::cause("external", "external"))?;
            open_session(store, session.as_deref())?;
            Ok(None)
        }
        "UserPromptSubmit" => {
            let lock = require(lock)?;
            let last = index::last_seq(&store.db)?;
            store.db.execute(
                "UPDATE spans SET end_seq = ?1, end_ns = ?2 WHERE kind = 'turn' AND end_seq IS NULL AND (?3 IS NULL OR session_id = ?3)",
                params![last, now_ns(), session],
            )?;
            store.db.execute("DELETE FROM intents", [])?;
            reconcile(store, lock, &EventMeta::cause("external", "external"))?;
            open_session(store, session.as_deref())?;
            let last = index::last_seq(&store.db)?;
            let number: i64 = store.db.query_row(
                "SELECT COALESCE(MAX(number), 0) + 1 FROM spans WHERE kind = 'turn'",
                [],
                |r| r.get(0),
            )?;
            let id = str_of(payload, "prompt_id")
                .map(String::from)
                .unwrap_or_else(|| format!("{}:{number}", session.as_deref().unwrap_or("session")));
            store.db.execute(
                "INSERT OR IGNORE INTO spans (id, kind, parent, session_id, number, start_seq, start_ns)
                 VALUES (?1, 'turn', ?2, ?2, ?3, ?4, ?5)",
                params![id, session, number, last, now_ns()],
            )?;
            Ok(None)
        }
        "PreToolUse" => {
            let tool = tool_name(payload);
            if tool == "Bash" {
                reconcile(
                    store,
                    require(lock)?,
                    &EventMeta::cause("external", "external"),
                )?;
            } else if EDIT_TOOLS.contains(&tool)
                && let (Some(rel), Some(id)) =
                    (tool_path(store, payload), str_of(payload, "tool_use_id"))
            {
                open_intent(store, &rel, id, str_of(payload, "agent_id"))?;
            }
            Ok(None)
        }
        "PostToolUse" => {
            let tool = tool_name(payload);
            if EDIT_TOOLS.contains(&tool) {
                let lock = require(lock)?;
                if let Some(rel) = tool_path(store, payload) {
                    record_edit(store, lock, payload, &rel, queued)?;
                }
            } else if tool == "Bash" {
                record_bash(store, require(lock)?, payload)?;
            }
            Ok(None)
        }
        "PostToolUseFailure" => {
            if let Some(id) = str_of(payload, "tool_use_id") {
                store
                    .db
                    .execute("DELETE FROM intents WHERE tool_use_id = ?1", [id])?;
            }
            Ok(None)
        }
        "SubagentStart" => {
            let Some(agent_id) = str_of(payload, "agent_id") else {
                return Ok(None);
            };
            let parent = current_turn(store, payload)?.map(|(id, _)| id);
            let last = index::last_seq(&store.db)?;
            store.db.execute(
                "INSERT OR REPLACE INTO spans (id, kind, parent, session_id, agent_type, start_seq, start_ns)
                 VALUES (?1, 'agent', ?2, ?3, ?4, ?5, ?6)",
                params![agent_id, parent, session, str_of(payload, "agent_type"), last, now_ns()],
            )?;
            Ok(None)
        }
        "SubagentStop" => {
            let Some(agent_id) = str_of(payload, "agent_id") else {
                return Ok(None);
            };
            let summary: Option<String> =
                str_of(payload, "last_assistant_message").map(|s| s.chars().take(4000).collect());
            let last = index::last_seq(&store.db)?;
            let updated = store.db.execute(
                "UPDATE spans SET end_seq = ?1, end_ns = ?2, summary = ?3 WHERE id = ?4 AND kind = 'agent'",
                params![last, now_ns(), summary, agent_id],
            )?;
            if updated == 0 {
                // Started before the hooks were installed: record what we know.
                let parent = current_turn(store, payload)?.map(|(id, _)| id);
                store.db.execute(
                    "INSERT OR IGNORE INTO spans (id, kind, parent, session_id, agent_type, start_seq, end_seq, end_ns, summary)
                     VALUES (?1, 'agent', ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
                    params![agent_id, parent, session, str_of(payload, "agent_type"), last, now_ns(), summary],
                )?;
            }
            Ok(None)
        }
        "Stop" => {
            let turn = current_turn(store, payload)?;
            let last = index::last_seq(&store.db)?;
            if let Some((id, _)) = &turn {
                store.db.execute(
                    "UPDATE spans SET end_seq = ?1, end_ns = ?2 WHERE id = ?3",
                    params![last, now_ns(), id],
                )?;
            }
            store.db.execute("DELETE FROM intents", [])?;
            if queued {
                return Ok(None);
            }
            let Some(offer) = crate::commit::offer(store, turn.map(|(_, n)| n))? else {
                return Ok(None);
            };
            let ask_agent = store.config.commit_offer == "ask-agent"
                && payload.get("stop_hook_active").and_then(Value::as_bool) != Some(true);
            if ask_agent {
                Ok(Some(
                    json!({"decision": "block", "reason": format!("{offer} Ask the user whether to commit; if yes, run `vs commit turn -m \"<message>\"`.")})
                        .to_string(),
                ))
            } else {
                Ok(Some(json!({"systemMessage": offer}).to_string()))
            }
        }
        _ => Ok(None),
    }
}

fn open_session(store: &Store, session: Option<&str>) -> Result<()> {
    let Some(session) = session else {
        return Ok(());
    };
    let last = index::last_seq(&store.db)?;
    store.db.execute(
        "INSERT OR IGNORE INTO spans (id, kind, session_id, start_seq, start_ns) VALUES (?1, 'session', ?1, ?2, ?3)",
        params![session, last, now_ns()],
    )?;
    Ok(())
}

/// Notes the file's content before an edit tool runs, so that a change made to
/// it in the meantime by someone else is not blamed on the agent.
fn open_intent(store: &Store, rel: &str, tool_use_id: &str, agent_id: Option<&str>) -> Result<()> {
    let abs = store.root.join(rel);
    let pre = match std::fs::symlink_metadata(&abs) {
        Ok(meta) if meta.is_file() || meta.file_type().is_symlink() => {
            let cached = scan::cached(store, rel)?;
            let stat = FileStat::of(&meta, i64::MAX);
            match cached {
                Some(c)
                    if !c.stat.racy && same(&c.stat, &stat) && c.entry.mode == mode_of(&meta) =>
                {
                    Some(c.entry)
                }
                _ => {
                    let content = read_recorded(&abs, &meta)?;
                    Some(Entry {
                        oid: odb::write_blob(store.repo()?, &content)?,
                        mode: mode_of(&meta),
                    })
                }
            }
        }
        _ => None,
    };
    store.db.execute(
        "INSERT OR REPLACE INTO intents (tool_use_id, path, opened_ns, pre_oid, pre_mode, agent_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![tool_use_id, rel, now_ns(), pre.map(|e| e.oid.to_string()), pre.map(|e| e.mode), agent_id],
    )?;
    Ok(())
}

fn same(a: &FileStat, b: &FileStat) -> bool {
    a.mtime_ns == b.mtime_ns && a.ctime_ns == b.ctime_ns && a.size == b.size && a.ino == b.ino
}

fn record_edit(
    store: &mut Store,
    lock: &WriteLock,
    payload: &Value,
    rel: &str,
    queued: bool,
) -> Result<()> {
    let tool_use_id = str_of(payload, "tool_use_id");
    let intent: Option<(Option<String>, Option<u32>)> = match tool_use_id {
        Some(id) => store
            .db
            .query_row(
                "SELECT pre_oid, pre_mode FROM intents WHERE tool_use_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?,
        None => None,
    };
    if let Some(id) = tool_use_id {
        store
            .db
            .execute("DELETE FROM intents WHERE tool_use_id = ?1", [id])?;
    }
    // A queued event is applied late, when the file may have moved on; the
    // pre-state check would then misfire, so it is only done live.
    if let (Some((pre_oid, pre_mode)), false) = (intent, queued) {
        let pre = match pre_oid {
            Some(oid) => Some(Entry {
                oid: odb::parse_oid(&oid)?,
                mode: pre_mode.unwrap_or(odb::MODE_FILE),
            }),
            None => None,
        };
        let recorded = scan::cached(store, rel)?.map(|c| c.entry);
        if pre != recorded {
            let change = ledger::FileChange {
                path: rel.to_string(),
                before: recorded,
                after: pre,
            };
            let unknown_stat = FileStat {
                mtime_ns: -1,
                ctime_ns: -1,
                size: -1,
                ino: -1,
                racy: true,
            };
            let row = CacheRow {
                path: rel.to_string(),
                value: pre.map(|e| (unknown_stat, e)),
            };
            let mut meta = meta_for(store, payload, "external", "external")?;
            meta.note = Some("changed before the agent's edit".into());
            ledger::record(store, lock, &meta, &[change], &[row])?;
        }
    }
    let mut oracle = new_oracle(store);
    let (changes, rows) = read_paths(store, &[rel.to_string()], &mut oracle)?;
    let meta = meta_for(store, payload, cause_of(tool_name(payload)), "agent")?;
    ledger::record(store, lock, &meta, &changes, &rows)?;
    Ok(())
}

fn record_bash(store: &mut Store, lock: &WriteLock, payload: &Value) -> Result<()> {
    let mut meta = meta_for(store, payload, "bash", "agent")?;
    let diff = payload
        .get("tool_response")
        .and_then(|r| r.get("bashEditDiff"));
    let listed: Option<Vec<String>> = diff.and_then(|d| {
        let flag = |k: &str| d.get(k).and_then(Value::as_bool).unwrap_or(false);
        if flag("unavailable") || flag("skipped") {
            return None;
        }
        let files = d.get("changedFiles")?.as_array()?;
        if files.is_empty() || files.len() >= 200 {
            return None;
        }
        let cwd = str_of(payload, "cwd")
            .map(PathBuf::from)
            .unwrap_or_else(|| store.root.clone());
        Some(
            files
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|p| resolve_from(&store.root, &cwd, Path::new(p)).ok())
                .filter(|p| !p.is_empty())
                .collect(),
        )
    });
    if diff.and_then(|d| d.get("shared")).and_then(Value::as_bool) == Some(true) {
        meta.ambiguous = Some("concurrent shell commands".into());
    }
    match listed {
        Some(paths) => {
            let mut oracle = new_oracle(store);
            let (changes, rows) = read_paths(store, &paths, &mut oracle)?;
            ledger::record(store, lock, &meta, &changes, &rows)?;
        }
        None => {
            reconcile(store, lock, &meta)?;
        }
    }
    Ok(())
}
