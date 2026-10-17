//! The ref grammar shared by every command.
//!
//! | Ref | State |
//! |---|---|
//! | `@` | the working tree now |
//! | `@-N` | N events back (`@-0` is the last event) |
//! | `e:N` | right after event N |
//! | `turn`, `turn-N`, `turn:N` | start of the latest turn, N turns before it, or turn number N |
//! | `session`, `session:<id>` | start of the latest (or a named) session |
//! | `agent:<type or id>` | start of that subagent's latest run |
//! | `t:14:05`, `t:-10m` | the last event at or before a time |
//! | `git:<rev>` | a git commit's tree |
//! | `<label>` | a label |
//!
//! `A..B` is a range, and `path@ref` names a file at a state.

use crate::error::{AppError, Result, git_err};
use crate::store::Store;
use crate::store::index::{self, Event, Span};
use crate::store::odb::{self, LABELS_PREFIX};
use chrono::{Local, NaiveTime, TimeZone};
use gix::ObjectId;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub enum Target {
    Worktree,
    State(State),
}

#[derive(Debug, Clone)]
pub struct State {
    pub tree: ObjectId,
    /// The event this state follows, when it is a point on the ledger.
    pub seq: Option<i64>,
    /// True if the state may not describe the whole tree (an imported v1
    /// checkpoint); then files it lacks are not deleted on restore.
    pub partial: bool,
    pub name: String,
}

impl Target {
    pub fn name(&self) -> String {
        match self {
            Target::Worktree => "@".into(),
            Target::State(s) => s.name.clone(),
        }
    }
}

fn usage(spec: &str, why: &str) -> AppError {
    AppError::Usage(format!("Cannot resolve '{spec}': {why}"))
}

pub fn is_label_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..")
}

/// The state right after event `seq` (seq 0: before anything was recorded).
pub fn state_at(store: &Store, seq: i64, name: String) -> Result<State> {
    if seq <= 0 {
        return Ok(State {
            tree: odb::empty_tree(),
            seq: Some(0),
            partial: false,
            name,
        });
    }
    let event = index::event(&store.db, seq)?.ok_or_else(|| {
        AppError::Usage(format!(
            "Event {seq} is not in the history (pruned by gc, or never recorded)"
        ))
    })?;
    Ok(State {
        tree: odb::parse_oid(&event.after_tree)?,
        seq: Some(seq),
        partial: false,
        name,
    })
}

pub fn resolve(store: &Store, spec: &str) -> Result<Target> {
    let spec = spec.trim();
    if spec == "@" {
        return Ok(Target::Worktree);
    }
    Ok(Target::State(resolve_state(store, spec)?))
}

/// The sequence number a ref points at on the ledger.
pub fn resolve_seq(store: &Store, spec: &str) -> Result<i64> {
    if spec.trim() == "@" {
        return index::last_seq(&store.db);
    }
    resolve_state(store, spec)?
        .seq
        .ok_or_else(|| usage(spec, "not a point in the history (a label or git commit)"))
}

pub fn resolve_state(store: &Store, spec: &str) -> Result<State> {
    let name = spec.to_string();
    let conn = &store.db;
    if let Some(n) = spec.strip_prefix("@-") {
        let n: i64 = n.parse().map_err(|_| usage(spec, "expected @-N"))?;
        let last = index::last_seq(conn)?;
        return state_at(store, nth_event_back(store, last, n)?, name);
    }
    if let Some(n) = spec.strip_prefix("e:") {
        let n: i64 = n.parse().map_err(|_| usage(spec, "expected e:N"))?;
        return state_at(store, n, name);
    }
    if spec == "turn" || spec.starts_with("turn-") || spec.starts_with("turn:") {
        let span = resolve_turn(store, spec)?;
        return state_at(store, span.start_seq, name);
    }
    if spec == "session" || spec.starts_with("session:") {
        let span = resolve_session(store, spec)?;
        return state_at(store, span.start_seq, name);
    }
    if let Some(agent) = spec.strip_prefix("agent:") {
        let start = match resolve_agent(store, agent) {
            Ok(span) => span.start_seq,
            Err(e) => conn
                .query_row(
                    "SELECT MIN(seq) - 1 FROM events WHERE agent_type = ?1 OR agent_id = ?1",
                    [agent],
                    |r| r.get::<_, Option<i64>>(0),
                )?
                .ok_or(e)?,
        };
        return state_at(store, start, name);
    }
    if let Some(time) = spec.strip_prefix("t:") {
        let ts = parse_time(time)
            .ok_or_else(|| usage(spec, "expected t:HH:MM, t:-10m or t:YYYY-MM-DDTHH:MM"))?;
        let seq: Option<i64> = conn
            .query_row("SELECT MAX(seq) FROM events WHERE ts_ns <= ?1", [ts], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        return state_at(store, seq.unwrap_or(0), name);
    }
    if let Some(rev) = spec.strip_prefix("git:") {
        return resolve_git(store, rev, name);
    }
    if spec.starts_with("lane:") {
        return Err(usage(spec, "lanes are not supported yet"));
    }
    if is_label_name(spec) {
        let repo = store.repo()?;
        if let Some(commit) = odb::read_ref(repo, &format!("{LABELS_PREFIX}{spec}"))? {
            let info = odb::read_commit(repo, commit)?;
            let trailers = odb::parse_trailers(&info.message);
            let seq = trailers.get("Vibesnap-Seq").and_then(|s| s.parse().ok());
            let partial = trailers
                .get("Vibesnap-Partial")
                .is_some_and(|v| v == "true");
            return Ok(State {
                tree: info.tree,
                seq,
                partial,
                name,
            });
        }
        return Err(usage(spec, "no such label or ref"));
    }
    Err(usage(spec, "unknown ref syntax"))
}

/// The seq `n` events before `last` (counting only events that still exist).
fn nth_event_back(store: &Store, last: i64, n: i64) -> Result<i64> {
    if n == 0 {
        return Ok(last);
    }
    let seq: Option<i64> = store
        .db
        .query_row(
            "SELECT seq FROM events WHERE seq <= ?1 ORDER BY seq DESC LIMIT 1 OFFSET ?2",
            [last, n],
            |r| r.get(0),
        )
        .optional()?;
    Ok(seq.unwrap_or(0))
}

pub fn resolve_turn(store: &Store, spec: &str) -> Result<Span> {
    let turns = index::spans(&store.db, "turn")?;
    let found = if spec == "turn" {
        turns.into_iter().next()
    } else if let Some(n) = spec.strip_prefix("turn-") {
        let n: usize = n.parse().map_err(|_| usage(spec, "expected turn-N"))?;
        turns.into_iter().nth(n)
    } else {
        let n: i64 = spec
            .trim_start_matches("turn:")
            .parse()
            .map_err(|_| usage(spec, "expected turn:N"))?;
        turns.into_iter().find(|t| t.number == Some(n))
    };
    found.ok_or_else(|| usage(spec, "no such turn recorded (are the hooks installed?)"))
}

pub fn resolve_session(store: &Store, spec: &str) -> Result<Span> {
    let sessions = index::spans(&store.db, "session")?;
    let found = match spec.strip_prefix("session:") {
        Some(id) => sessions.into_iter().find(|s| s.id.starts_with(id)),
        None => sessions.into_iter().next(),
    };
    found.ok_or_else(|| usage(spec, "no such session recorded (are the hooks installed?)"))
}

pub fn resolve_agent(store: &Store, agent: &str) -> Result<Span> {
    index::spans(&store.db, "agent")?
        .into_iter()
        .find(|s| s.id == agent || s.agent_type.as_deref() == Some(agent))
        .ok_or_else(|| {
            usage(
                &format!("agent:{agent}"),
                "no subagent run with that type or id",
            )
        })
}

fn resolve_git(store: &Store, rev: &str, name: String) -> Result<State> {
    let rev = if rev.is_empty() { "HEAD" } else { rev };
    let project = gix::open_opts(&store.root, gix::open::Options::isolated()).map_err(git_err)?;
    let id = project.rev_parse_single(rev).map_err(git_err)?;
    let tree = id
        .object()
        .map_err(git_err)?
        .peel_to_tree()
        .map_err(git_err)?
        .id;
    Ok(State {
        tree,
        seq: None,
        partial: false,
        name,
    })
}

/// Nanoseconds since the epoch for `14:05`, `14:05:30`, `-10m`, `-2h`, `-1d`,
/// `-30s` or `2026-10-16T14:05`.
pub fn parse_time(text: &str) -> Option<i64> {
    let now = Local::now();
    if let Some(rel) = text.strip_prefix('-') {
        let (num, unit) = rel.split_at(rel.find(|c: char| !c.is_ascii_digit())?);
        let n: i64 = num.parse().ok()?;
        let secs = match unit {
            "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86400,
            _ => return None,
        };
        return now.timestamp_nanos_opt().map(|t| t - secs * 1_000_000_000);
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S"))
    {
        return Local
            .from_local_datetime(&dt)
            .single()?
            .timestamp_nanos_opt();
    }
    let time = NaiveTime::parse_from_str(text, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
        .ok()?;
    let mut dt = Local
        .from_local_datetime(&now.date_naive().and_time(time))
        .single()?;
    if dt > now {
        dt -= chrono::Duration::days(1);
    }
    dt.timestamp_nanos_opt()
}

/// Splits `path@ref`. The ref is after the last `@`; `path@-3` means `path@@-3`
/// and `path@@` is the working tree.
pub fn split_path_ref(spec: &str) -> Option<(String, String)> {
    if let Some(path) = spec.strip_suffix("@@").filter(|p| !p.is_empty()) {
        return Some((path.to_string(), "@".into()));
    }
    let at = spec.rfind('@')?;
    let (path, rest) = (&spec[..at], &spec[at + 1..]);
    if path.is_empty() || rest.is_empty() {
        return None;
    }
    if path.ends_with('@') && rest.starts_with('-') {
        return Some((path[..path.len() - 1].to_string(), format!("@{rest}")));
    }
    if let Some(n) = rest.strip_prefix('-') {
        if n.parse::<u64>().is_ok() {
            return Some((path.to_string(), format!("@{rest}")));
        }
        return None;
    }
    let plausible = ["e:", "turn", "session", "agent:", "t:", "git:", "lane:"]
        .iter()
        .any(|p| rest.starts_with(p))
        || is_label_name(rest);
    plausible.then(|| (path.to_string(), rest.to_string()))
}

/// The events a spec selects, oldest first: one event (`e:N`), a subagent's
/// run (`agent:X`), a turn, a session, a range `A..B`, or everything after a
/// state ref.
pub fn resolve_events(store: &Store, spec: &str) -> Result<Vec<Event>> {
    let conn = &store.db;
    let last = index::last_seq(conn)?;
    if let Some((a, b)) = spec.split_once("..") {
        let from = resolve_seq(store, if a.is_empty() { "session" } else { a })?;
        let to = if b.is_empty() {
            last
        } else {
            resolve_seq(store, b)?
        };
        return index::events_between(conn, from, to);
    }
    if let Some(n) = spec.strip_prefix("e:") {
        let n: i64 = n.parse().map_err(|_| usage(spec, "expected e:N"))?;
        return index::event(conn, n)?
            .map(|e| vec![e])
            .ok_or_else(|| usage(spec, "no such event"));
    }
    if let Some(agent) = spec.strip_prefix("agent:") {
        let Ok(span) = resolve_agent(store, agent) else {
            // No SubagentStart was seen (hooks installed mid-run): match by tag.
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM events WHERE agent_type = ?1 OR agent_id = ?1 ORDER BY seq",
                index::EVENT_COLUMNS
            ))?;
            let rows = stmt.query_map([agent], index::event_from_row)?;
            let events: Vec<Event> = rows.collect::<rusqlite::Result<_>>()?;
            if events.is_empty() {
                return Err(usage(spec, "no events by a subagent with that type or id"));
            }
            return Ok(events);
        };
        let end = span.end_seq.unwrap_or(last);
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM events WHERE agent_id = ?1 AND seq > ?2 AND seq <= ?3 ORDER BY seq",
            index::EVENT_COLUMNS
        ))?;
        let rows = stmt.query_map(
            rusqlite::params![span.id, span.start_seq, end],
            index::event_from_row,
        )?;
        return Ok(rows.collect::<rusqlite::Result<_>>()?);
    }
    if spec == "turn" || spec.starts_with("turn-") || spec.starts_with("turn:") {
        let span = resolve_turn(store, spec)?;
        return index::events_between(conn, span.start_seq, span.end_seq.unwrap_or(last));
    }
    let from = resolve_seq(store, spec)?;
    index::events_between(conn, from, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_path_refs() {
        assert_eq!(
            split_path_ref("src/a.ts@turn"),
            Some(("src/a.ts".into(), "turn".into()))
        );
        assert_eq!(
            split_path_ref("src/a.ts@-3"),
            Some(("src/a.ts".into(), "@-3".into()))
        );
        assert_eq!(
            split_path_ref("src/a.ts@@-3"),
            Some(("src/a.ts".into(), "@-3".into()))
        );
        assert_eq!(
            split_path_ref("src/a.ts@e:12"),
            Some(("src/a.ts".into(), "e:12".into()))
        );
        assert_eq!(
            split_path_ref("node_modules/@types/x.d.ts@t:14:05"),
            Some(("node_modules/@types/x.d.ts".into(), "t:14:05".into()))
        );
        assert_eq!(split_path_ref("node_modules/@types/x.d.ts"), None);
        assert_eq!(split_path_ref("a.ts@@"), Some(("a.ts".into(), "@".into())));
        assert_eq!(
            split_path_ref("a.ts@before-refactor"),
            Some(("a.ts".into(), "before-refactor".into()))
        );
    }

    #[test]
    fn parses_times() {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap();
        let ten = parse_time("-10m").unwrap();
        assert!((now - ten - 600_000_000_000).abs() < 5_000_000_000);
        assert!(parse_time("14:05").unwrap() <= now);
        assert!(parse_time("2026-10-16T14:05").is_some());
        assert!(parse_time("soon").is_none());
    }
}
