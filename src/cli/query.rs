//! Read-only commands: diff, log, show, blame, recap, overlaps.

use super::out::{Budget, Out, clock_of, time_of};
use super::record::{recorded_map, resolve_scope};
use crate::error::{AppError, Result};
use crate::paths::{in_scope, resolve_user_path};
use crate::refs::{self, Target};
use crate::store::Store;
use crate::store::index::{self, Event, PathRow};
use crate::store::odb::{self, Entry};
use crate::text::{self, human_size, is_binary};
use crate::wc::scan;
use colored::Colorize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The files of a side, by path.
fn side_files(store: &Store, side: &Target, scope: &[String]) -> Result<BTreeMap<String, Entry>> {
    match side {
        Target::State(s) => odb::flatten_scoped(store.repo()?, s.tree, scope),
        Target::Worktree => {
            let mut map = recorded_map(store)?;
            let result = scan::scan(store, &Default::default(), false)?;
            for c in result.changes {
                match c.after {
                    Some(e) => map.insert(c.path, e),
                    None => map.remove(&c.path),
                };
            }
            map.retain(|p, _| in_scope(p, scope));
            Ok(map)
        }
    }
}

pub struct FileDiff {
    pub path: String,
    pub before: Option<Entry>,
    pub after: Option<Entry>,
}

fn changed_files(
    store: &Store,
    from: &Target,
    to: &Target,
    scope: &[String],
) -> Result<Vec<FileDiff>> {
    let mut out = Vec::new();
    if let (Target::State(a), Target::State(b)) = (from, to) {
        for c in odb::diff_trees(store.repo()?, a.tree, b.tree)? {
            if in_scope(&c.path, scope) {
                out.push(FileDiff {
                    path: c.path,
                    before: c.before,
                    after: c.after,
                });
            }
        }
    } else {
        let a = side_files(store, from, scope)?;
        let b = side_files(store, to, scope)?;
        let paths: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        for path in paths {
            let (x, y) = (a.get(path).copied(), b.get(path).copied());
            if x != y {
                out.push(FileDiff {
                    path: path.clone(),
                    before: x,
                    after: y,
                });
            }
        }
    }
    let partial = |t: &Target| matches!(t, Target::State(s) if s.partial);
    out.retain(|d| !(d.before.is_none() && partial(from)) && !(d.after.is_none() && partial(to)));
    // A file missing from the working-tree side but present on disk is one
    // vibesnap never records (ignored, a secret, too large), not a deletion.
    let exists = |p: &str| fs::symlink_metadata(store.root.join(p)).is_ok();
    out.retain(|d| {
        !(matches!(to, Target::Worktree) && d.after.is_none() && exists(&d.path))
            && !(matches!(from, Target::Worktree) && d.before.is_none() && exists(&d.path))
    });
    Ok(out)
}

/// The bytes of `entry` on `side` (the working tree is read from disk).
fn content(store: &Store, side: &Target, path: &str, entry: Option<Entry>) -> Result<Vec<u8>> {
    let Some(entry) = entry else {
        return Ok(Vec::new());
    };
    match side {
        Target::Worktree => {
            let abs = store.root.join(path);
            let meta = fs::symlink_metadata(&abs)?;
            Ok(scan::read_recorded(&abs, &meta)?)
        }
        Target::State(_) => odb::read_blob(store.repo()?, entry.oid),
    }
}

fn status_of(d: &FileDiff) -> &'static str {
    match (d.before, d.after) {
        (None, _) => "added",
        (_, None) => "deleted",
        _ => "modified",
    }
}

fn seq_of(store: &Store, t: &Target) -> Result<Option<i64>> {
    Ok(match t {
        Target::Worktree => Some(index::last_seq(&store.db)?),
        Target::State(s) => s.seq,
    })
}

/// Who changed `path` in `from < seq <= to`, and in which events.
fn authors(store: &Store, path: &str, from: i64, to: i64) -> Result<(Vec<String>, Vec<i64>)> {
    let mut stmt = store.db.prepare_cached(&format!(
        "SELECT {} FROM events WHERE seq IN (SELECT seq FROM event_paths WHERE path = ?1 AND seq > ?2 AND seq <= ?3) ORDER BY seq",
        index::EVENT_COLUMNS
    ))?;
    let events: Vec<Event> = stmt
        .query_map(rusqlite::params![path, from, to], index::event_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut who: Vec<String> = Vec::new();
    for e in &events {
        let w = e.who();
        if !who.contains(&w) {
            who.push(w);
        }
    }
    Ok((who, events.iter().map(|e| e.seq).collect()))
}

pub fn diff(
    out: &Out,
    refs_in: &[String],
    stat: bool,
    name_only: bool,
    context: usize,
    max_lines: usize,
    paths: &[PathBuf],
) -> Result<()> {
    let store = Store::find()?;
    let scope = resolve_scope(&store, paths)?;
    let (from_spec, to_spec) = match refs_in {
        [] => {
            let has_turn = !index::spans(&store.db, "turn")?.is_empty();
            (
                if has_turn {
                    "turn".to_string()
                } else {
                    "@-0".to_string()
                },
                "@".to_string(),
            )
        }
        [one] => match one.split_once("..") {
            Some((a, b)) => (
                if a.is_empty() {
                    "session".into()
                } else {
                    a.to_string()
                },
                if b.is_empty() {
                    "@".into()
                } else {
                    b.to_string()
                },
            ),
            None => (one.clone(), "@".into()),
        },
        [a, b] => (a.clone(), b.clone()),
        _ => return Err(AppError::Usage("diff takes at most two refs".into())),
    };
    let from = refs::resolve(&store, &from_spec)?;
    let to = refs::resolve(&store, &to_spec)?;
    let files = changed_files(&store, &from, &to, &scope)?;
    let seqs = (seq_of(&store, &from)?, seq_of(&store, &to)?);

    let mut rows = Vec::new();
    let mut budget = Budget::new(max_lines);
    let mut truncated = false;
    let (mut total_add, mut total_del) = (0, 0);
    for d in &files {
        let old = content(&store, &from, &d.path, d.before)?;
        let new = content(&store, &to, &d.path, d.after)?;
        let binary = is_binary(&old) || is_binary(&new);
        let (add, del) = text::line_counts(&old, &new).unwrap_or((0, 0));
        total_add += add;
        total_del += del;
        let (agents, events) = match seqs {
            (Some(a), Some(b)) if a < b => authors(&store, &d.path, a, b)?,
            _ => (Vec::new(), Vec::new()),
        };
        let mut row = json!({
            "path": d.path, "status": status_of(d), "add": add, "del": del, "binary": binary,
            "agents": agents, "events": events,
        });
        if !stat && !name_only {
            let patch = if binary {
                format!(
                    "Binary file {}: {} -> {}\n",
                    d.path,
                    human_size(old.len()),
                    human_size(new.len())
                )
            } else {
                text::unified(&old, &new, &d.path, &from_spec, &to_spec, context)
            };
            if out.json {
                let lines = patch.lines().count();
                if budget.used + lines > budget.max {
                    truncated = true;
                } else {
                    budget.used += lines;
                    row["patch"] = patch.into();
                }
            } else {
                for line in patch.lines() {
                    let colored = if line.starts_with("+++") || line.starts_with("---") {
                        line.bold().to_string()
                    } else if line.starts_with('+') {
                        line.green().to_string()
                    } else if line.starts_with('-') {
                        line.red().to_string()
                    } else if line.starts_with("@@") {
                        line.cyan().to_string()
                    } else {
                        line.to_string()
                    };
                    budget.line(&colored);
                }
            }
        }
        rows.push(row);
    }

    if out.json {
        out.emit(json!({
            "from": from.name(), "to": to.name(), "files": rows,
            "add": total_add, "del": total_del, "truncated": truncated,
        }));
        return Ok(());
    }
    if files.is_empty() {
        println!("No changes between {} and {}.", from.name(), to.name());
        return Ok(());
    }
    if name_only {
        for d in &files {
            let letter = match status_of(d) {
                "added" => "A",
                "deleted" => "D",
                _ => "M",
            };
            println!("{letter} {}", d.path);
        }
    } else if stat {
        let width = files
            .iter()
            .map(|d| d.path.len())
            .max()
            .unwrap_or(0)
            .min(60);
        for row in &rows {
            let agents: Vec<&str> = row["agents"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let change = if row["binary"] == true {
                "binary".to_string()
            } else {
                format!("+{} -{}", row["add"], row["del"])
            };
            println!(
                "{:width$}  {:>12}  {}",
                row["path"].as_str().unwrap_or_default(),
                change,
                agents.join(", ")
            );
        }
        println!(
            "{} files changed, +{total_add} -{total_del} ({} -> {})",
            files.len(),
            from.name(),
            to.name()
        );
    } else {
        budget.finish("pass --max-lines, --stat, or narrow with -- <path>");
    }
    Ok(())
}

pub struct LogArgs {
    pub spec: Option<String>,
    pub agent: Option<String>,
    pub actor: Option<String>,
    pub since: Option<String>,
    pub search: Option<String>,
    pub stat: bool,
    pub limit: usize,
    pub paths: Vec<PathBuf>,
}

pub fn log(out: &Out, args: LogArgs) -> Result<()> {
    let store = Store::find()?;
    let scope = resolve_scope(&store, &args.paths)?;
    let mut sql = format!("SELECT {} FROM events WHERE 1 = 1", index::EVENT_COLUMNS);
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(spec) = &args.spec {
        let selected = refs::resolve_events(&store, spec)?;
        let list: Vec<String> = selected.iter().map(|e| e.seq.to_string()).collect();
        sql.push_str(&format!(
            " AND seq IN ({})",
            if list.is_empty() {
                "NULL".into()
            } else {
                list.join(",")
            }
        ));
    }
    if let Some(since) = &args.since {
        sql.push_str(" AND seq > ?");
        params.push(refs::resolve_seq(&store, since)?.into());
    }
    if let Some(actor) = &args.actor {
        sql.push_str(" AND actor = ?");
        params.push(actor.clone().into());
    }
    if let Some(agent) = &args.agent {
        if agent == "main" {
            sql.push_str(" AND actor = 'agent' AND agent_id IS NULL");
        } else {
            sql.push_str(" AND (agent_type = ? OR agent_id = ?)");
            params.push(agent.clone().into());
            params.push(agent.clone().into());
        }
    }
    if !scope.is_empty() {
        let mut ors = Vec::new();
        for prefix in &scope {
            ors.push("path = ? OR (path >= ? AND path < ?)".to_string());
            params.push(prefix.clone().into());
            params.push(format!("{prefix}/").into());
            params.push(format!("{prefix}0").into());
        }
        sql.push_str(&format!(
            " AND seq IN (SELECT seq FROM event_paths WHERE {})",
            ors.join(" OR ")
        ));
    }
    sql.push_str(" ORDER BY seq DESC");
    let mut stmt = store.db.prepare(&sql)?;
    let candidates = stmt.query_map(rusqlite::params_from_iter(params), index::event_from_row)?;

    let mut shown = 0;
    for event in candidates {
        if shown >= args.limit {
            break;
        }
        let event = event?;
        let files: Vec<PathRow> = index::event_paths(&store.db, event.seq)?
            .into_iter()
            .filter(|r| in_scope(&r.path, &scope))
            .collect();
        if let Some(needle) = &args.search
            && !touches_string(&store, &files, needle)?
        {
            continue;
        }
        shown += 1;
        print_event(out, &event, &files, args.stat);
    }
    if shown == 0 && !out.json {
        println!("No matching events.");
    }
    Ok(())
}

/// True if the number of occurrences of `needle` changed in any of the files.
fn touches_string(store: &Store, files: &[PathRow], needle: &str) -> Result<bool> {
    let repo = store.repo()?;
    let count = |oid: &Option<String>| -> Result<usize> {
        match oid {
            None => Ok(0),
            Some(oid) => {
                let data = odb::read_blob(repo, odb::parse_oid(oid)?)?;
                Ok(String::from_utf8_lossy(&data).matches(needle).count())
            }
        }
    };
    for f in files {
        if count(&f.before_oid)? != count(&f.after_oid)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn file_summary(files: &[PathRow]) -> String {
    let (add, del): (i64, i64) = files.iter().fold((0, 0), |(a, d), f| {
        (a + f.added.unwrap_or(0), d + f.removed.unwrap_or(0))
    });
    let names = match files.len() {
        0 => "(no files)".to_string(),
        1 => files[0].path.clone(),
        n => format!("{} and {} more", files[0].path, n - 1),
    };
    format!("{names} (+{add} -{del})")
}

fn print_event(out: &Out, e: &Event, files: &[PathRow], stat: bool) {
    if out.json {
        let mut value = serde_json::to_value(e).unwrap_or_default();
        value["who"] = e.who().into();
        value["files"] = files
            .iter()
            .map(|f| json!({"path": f.path, "add": f.added, "del": f.removed,
                "status": match (&f.before_oid, &f.after_oid) { (None, _) => "added", (_, None) => "deleted", _ => "modified" }}))
            .collect();
        out.emit(value);
        return;
    }
    let turn = e
        .turn
        .map(|t| format!("turn:{t}"))
        .unwrap_or_else(|| "-".into());
    println!(
        "{}  {}  {:8}  {:14}  {:8}  {}{}",
        format!("e:{}", e.seq).yellow(),
        time_of(e.ts_ns),
        turn,
        e.who(),
        e.cause,
        file_summary(files),
        e.note
            .as_ref()
            .map(|n| format!("  \"{}\"", n.lines().next().unwrap_or_default()))
            .unwrap_or_default()
    );
    if stat {
        for f in files {
            println!(
                "      {}  +{} -{}",
                f.path,
                f.added.unwrap_or(0),
                f.removed.unwrap_or(0)
            );
        }
    }
}

pub fn show(out: &Out, spec: &str, patch: bool, max_lines: usize) -> Result<()> {
    let store = Store::find()?;
    if let Some(n) = spec.strip_prefix("e:") {
        let seq: i64 = n
            .parse()
            .map_err(|_| AppError::Usage("expected e:N".into()))?;
        let event = index::event(&store.db, seq)?
            .ok_or_else(|| AppError::Usage(format!("No event e:{seq}")))?;
        let files = index::event_paths(&store.db, seq)?;
        if out.json && !patch {
            print_event(out, &event, &files, true);
            return Ok(());
        }
        let mut patches = Vec::new();
        if patch {
            let repo = store.repo()?;
            let read = |oid: &Option<String>| -> Result<Vec<u8>> {
                match oid {
                    Some(o) => odb::read_blob(repo, odb::parse_oid(o)?),
                    None => Ok(Vec::new()),
                }
            };
            for f in &files {
                let (old, new) = (read(&f.before_oid)?, read(&f.after_oid)?);
                patches.push(if is_binary(&old) || is_binary(&new) {
                    format!(
                        "Binary file {}: {} -> {}\n",
                        f.path,
                        human_size(old.len()),
                        human_size(new.len())
                    )
                } else {
                    text::unified(
                        &old,
                        &new,
                        &f.path,
                        &format!("e:{}", seq - 1),
                        &format!("e:{seq}"),
                        3,
                    )
                });
            }
        }
        if out.json {
            let mut value = serde_json::to_value(&event).unwrap_or_default();
            value["who"] = event.who().into();
            value["files"] = files
                .iter()
                .zip(&patches)
                .map(|(f, p)| json!({"path": f.path, "add": f.added, "del": f.removed, "patch": p}))
                .collect();
            out.emit(value);
            return Ok(());
        }
        println!(
            "{}  {}",
            format!("e:{}", event.seq).yellow(),
            time_of(event.ts_ns)
        );
        println!(
            "  cause: {}  by: {}  actor: {}",
            event.cause,
            event.who(),
            event.actor
        );
        if let Some(t) = event.turn {
            println!("  turn: {t}");
        }
        if let Some(s) = &event.session_id {
            println!("  session: {s}");
        }
        if let Some(t) = &event.tool {
            println!(
                "  tool: {t}{}",
                event
                    .tool_use_id
                    .as_ref()
                    .map(|u| format!(" ({u})"))
                    .unwrap_or_default()
            );
        }
        if let Some(a) = &event.ambiguous {
            println!("  ambiguous: {a}");
        }
        if let Some(n) = &event.note {
            println!("  note: {n}");
        }
        println!("  commit: {}", event.commit_oid);
        for f in &files {
            println!(
                "  {}  +{} -{}",
                f.path,
                f.added.unwrap_or(0),
                f.removed.unwrap_or(0)
            );
        }
        let mut budget = Budget::new(max_lines);
        for p in patches {
            budget.block(&p);
        }
        budget.finish("pass --max-lines");
        return Ok(());
    }

    let (path, r) = refs::split_path_ref(spec)
        .ok_or_else(|| AppError::Usage(format!("show takes path@ref or e:N, got '{spec}'")))?;
    let rel = resolve_user_path(&store.root, Path::new(&path))?;
    let target = refs::resolve(&store, &r)?;
    let data = match &target {
        Target::Worktree => {
            let abs = store.root.join(&rel);
            let meta = fs::symlink_metadata(&abs)
                .map_err(|_| AppError::Usage(format!("{rel} does not exist now")))?;
            scan::read_recorded(&abs, &meta)?
        }
        Target::State(s) => {
            let entry = odb::entry_at(store.repo()?, s.tree, &rel)?
                .ok_or_else(|| AppError::Usage(format!("{rel} does not exist at {r}")))?;
            odb::read_blob(store.repo()?, entry.oid)?
        }
    };
    let binary = is_binary(&data);
    if out.json {
        let text = String::from_utf8_lossy(&data);
        let lines: Vec<&str> = text.lines().collect();
        let truncated = !binary && lines.len() > max_lines;
        out.emit(json!({
            "path": rel, "ref": r, "size": data.len(), "binary": binary, "truncated": truncated,
            "content": if binary { Value::Null } else { lines[..lines.len().min(max_lines)].join("\n").into() },
        }));
        return Ok(());
    }
    if binary {
        println!("binary file {rel} at {r}, {}", human_size(data.len()));
        return Ok(());
    }
    // Print the bytes as stored (keeping \r\n and other encodings), up to the budget.
    let mut stdout = std::io::stdout().lock();
    let mut lines = data.split_inclusive(|&b| b == b'\n');
    for line in lines.by_ref().take(max_lines) {
        stdout.write_all(line)?;
    }
    let rest = lines.count();
    if !data.ends_with(b"\n") && !data.is_empty() {
        writeln!(stdout)?;
    }
    if rest > 0 {
        writeln!(stdout, "... {rest} more lines not shown (pass --max-lines)")?;
    }
    Ok(())
}

/// Line origins after replaying `history` from `start`: one seq per line
/// (-1: older than the recorded history, 0: not recorded yet).
fn attribute(
    store: &Store,
    history: &[PathRow],
    worktree: Option<&[u8]>,
) -> Result<(String, Vec<i64>)> {
    let repo = store.repo()?;
    let read = |oid: &Option<String>| -> Result<String> {
        match oid {
            Some(o) => Ok(
                String::from_utf8_lossy(&odb::read_blob(repo, odb::parse_oid(o)?)?).into_owned(),
            ),
            None => Ok(String::new()),
        }
    };
    let mut text = match history.first() {
        Some(first) => read(&first.before_oid)?,
        None => String::new(),
    };
    let mut origin = vec![-1; text::count_lines(&text)];
    let step = |text: &mut String, origin: &mut Vec<i64>, new: String, seq: i64| {
        let mut next = Vec::with_capacity(text::count_lines(&new));
        for (old_range, new_range, kind) in text::line_ops(text, &new) {
            if kind == 'e' {
                next.extend_from_slice(&origin[old_range]);
            } else {
                next.extend(std::iter::repeat_n(seq, new_range.len()));
            }
        }
        *origin = next;
        *text = new;
    };
    for row in history {
        let new = read(&row.after_oid)?;
        step(&mut text, &mut origin, new, row.seq);
    }
    if let Some(disk) = worktree {
        let disk = String::from_utf8_lossy(disk).into_owned();
        if disk != text {
            step(&mut text, &mut origin, disk, 0);
        }
    }
    Ok((text, origin))
}

pub fn blame(out: &Out, path: &Path, lines: Option<&str>) -> Result<()> {
    let store = Store::find()?;
    let rel = resolve_user_path(&store.root, path)?;
    let last = index::last_seq(&store.db)?;
    let history: Vec<PathRow> = index::path_history(&store.db, &rel, 0, last)?
        .into_iter()
        .filter(|r| r.path == rel)
        .collect();
    let disk = fs::read(store.root.join(&rel)).ok();
    if history.is_empty() && disk.is_none() {
        return Err(AppError::Usage(format!("{rel} has no recorded history")));
    }
    if disk.as_deref().is_some_and(is_binary) {
        return Err(AppError::Usage(format!("{rel} is binary")));
    }
    let (_, origin) = attribute(&store, &history, disk.as_deref())?;
    let (lo, hi) = match lines {
        Some(range) => {
            let (a, b) = range.split_once(',').unwrap_or((range, range));
            let a: usize = a
                .trim()
                .parse()
                .map_err(|_| AppError::Usage("-L expects a,b".into()))?;
            let b: usize = b
                .trim()
                .parse()
                .map_err(|_| AppError::Usage("-L expects a,b".into()))?;
            (a.max(1), b.max(a))
        }
        None => (1, origin.len().max(1)),
    };
    let mut events: HashMap<i64, Option<Event>> = HashMap::new();
    let mut hunks = Vec::new();
    let mut i = lo;
    while i <= hi.min(origin.len()) {
        let seq = origin[i - 1];
        let mut j = i;
        while j < hi.min(origin.len()) && origin[j] == seq {
            j += 1;
        }
        let event = events
            .entry(seq)
            .or_insert_with(|| {
                if seq > 0 {
                    index::event(&store.db, seq).ok().flatten()
                } else {
                    None
                }
            })
            .clone();
        hunks.push((i, j, seq, event));
        i = j + 1;
    }
    if out.json {
        out.emit(json!({
            "path": rel,
            "hunks": hunks.iter().map(|(a, b, seq, e)| json!({
                "start": a, "end": b,
                "event": if *seq > 0 { json!(seq) } else { Value::Null },
                "state": match seq { 0 => "not recorded yet", -1 => "older than the history", _ => "recorded" },
                "who": e.as_ref().map(|e| e.who()), "ts_ns": e.as_ref().map(|e| e.ts_ns),
                "turn": e.as_ref().and_then(|e| e.turn), "cause": e.as_ref().map(|e| e.cause.clone()),
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    for (a, b, seq, e) in hunks {
        let range = format!("{a}-{b}");
        match (seq, e) {
            (0, _) => println!("{range:9}  (not recorded yet)"),
            (-1, _) => println!("{range:9}  (older than the recorded history)"),
            (_, Some(e)) => println!(
                "{range:9}  {:8}  {:14}  {}  {}",
                format!("e:{}", e.seq),
                e.who(),
                clock_of(e.ts_ns),
                e.turn
                    .map(|t| format!("turn:{t}"))
                    .unwrap_or_else(|| e.cause.clone())
            ),
            (seq, None) => println!("{range:9}  e:{seq} (pruned)"),
        }
    }
    Ok(())
}

struct TurnSummary {
    number: Option<i64>,
    start_ns: Option<i64>,
    end_ns: Option<i64>,
    files: BTreeMap<String, (i64, i64)>,
    agents: Vec<String>,
    subagents: Vec<(String, String)>,
}

fn summarize(store: &Store, span: &index::Span, last: i64) -> Result<TurnSummary> {
    let events = index::events_between(&store.db, span.start_seq, span.end_seq.unwrap_or(last))?;
    let mut files: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut agents = Vec::new();
    for e in &events {
        if e.actor == "agent" && !agents.contains(&e.who()) {
            agents.push(e.who());
        }
        for f in index::event_paths(&store.db, e.seq)? {
            let entry = files.entry(f.path).or_default();
            entry.0 += f.added.unwrap_or(0);
            entry.1 += f.removed.unwrap_or(0);
        }
    }
    let mut stmt = store.db.prepare_cached(
        "SELECT agent_type, summary FROM spans WHERE kind = 'agent' AND parent = ?1 ORDER BY start_seq",
    )?;
    let subagents = stmt
        .query_map([&span.id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .map(|(t, s)| {
            (
                t.unwrap_or_else(|| "subagent".into()),
                s.unwrap_or_default(),
            )
        })
        .collect();
    Ok(TurnSummary {
        number: span.number,
        start_ns: span.start_ns,
        end_ns: span.end_ns,
        files,
        agents,
        subagents,
    })
}

/// Lines of the recap, newest turns kept when over budget.
pub fn recap_text(
    store: &Store,
    last_turn_only: bool,
    max_lines: usize,
) -> Result<(Vec<String>, Value)> {
    let last = index::last_seq(&store.db)?;
    let session = index::spans(&store.db, "session")?.into_iter().next();
    let mut turns: Vec<index::Span> = index::spans(&store.db, "turn")?
        .into_iter()
        .filter(|t| {
            session
                .as_ref()
                .is_none_or(|s| t.session_id.as_deref() == Some(s.id.as_str()))
        })
        .collect();
    if last_turn_only {
        turns.truncate(1);
    }
    turns.reverse();
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut json_turns = Vec::new();
    for span in &turns {
        let s = summarize(store, span, last)?;
        let (add, del) = s
            .files
            .values()
            .fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
        let when = match (s.start_ns, s.end_ns) {
            (Some(a), Some(b)) => format!("{}-{}", clock_of(a), clock_of(b)),
            (Some(a), None) => format!("{}-now", clock_of(a)),
            _ => String::new(),
        };
        let mut block = vec![format!(
            "turn:{} {when}  {} files (+{add} -{del}){}",
            s.number.unwrap_or_default(),
            s.files.len(),
            if s.agents.is_empty() {
                String::new()
            } else {
                format!("  by {}", s.agents.join(", "))
            }
        )];
        if !s.files.is_empty() {
            let mut names: Vec<String> = s
                .files
                .iter()
                .take(6)
                .map(|(p, (a, d))| format!("{p} +{a} -{d}"))
                .collect();
            if s.files.len() > 6 {
                names.push(format!("and {} more", s.files.len() - 6));
            }
            block.push(format!("  {}", names.join(", ")));
        }
        for (kind, summary) in &s.subagents {
            let summary: String = summary.split_whitespace().collect::<Vec<_>>().join(" ");
            let short: String = summary.chars().take(160).collect();
            block.push(format!(
                "  {kind}: {short}{}",
                if summary.chars().count() > 160 {
                    "..."
                } else {
                    ""
                }
            ));
        }
        json_turns.push(json!({
            "number": s.number, "start_ns": s.start_ns, "end_ns": s.end_ns, "agents": s.agents,
            "files": s.files.iter().map(|(p, (a, d))| json!({"path": p, "add": a, "del": d})).collect::<Vec<_>>(),
            "subagents": s.subagents.iter().map(|(t, s)| json!({"type": t, "summary": s})).collect::<Vec<_>>(),
        }));
        blocks.push(block);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut dropped = 0;
    for block in blocks.iter().rev() {
        if lines.len() + block.len() > max_lines.saturating_sub(1) {
            dropped += 1;
            continue;
        }
        let mut b = block.clone();
        b.extend(lines);
        lines = b;
    }
    if dropped > 0 {
        lines.insert(0, format!("({dropped} earlier turns not shown)"));
    }
    if turns.is_empty() {
        let events = index::events_between(&store.db, (last - 10).max(0), last)?;
        lines.push("No turns recorded (are the hooks installed?). Last events:".into());
        for e in events.iter().rev() {
            let files = index::event_paths(&store.db, e.seq)?;
            lines.push(format!(
                "  e:{} {} {} {}",
                e.seq,
                clock_of(e.ts_ns),
                e.who(),
                file_summary(&files)
            ));
        }
    }
    let value = json!({"session": session.map(|s| s.id), "turns": json_turns});
    Ok((lines, value))
}

pub fn recap(out: &Out, turn: bool, max_lines: usize) -> Result<()> {
    let store = Store::find()?;
    let (lines, value) = recap_text(&store, turn, max_lines)?;
    if out.json {
        out.emit(value);
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

pub fn overlaps(out: &Out, since: &str) -> Result<()> {
    let store = Store::find()?;
    let from = refs::resolve_seq(&store, since)?;
    let last = index::last_seq(&store.db)?;
    let events: HashMap<i64, Event> = index::events_between(&store.db, from, last)?
        .into_iter()
        .map(|e| (e.seq, e))
        .collect();
    let mut by_path: BTreeMap<String, Vec<PathRow>> = BTreeMap::new();
    for row in index::path_history(&store.db, "", from, last)? {
        by_path.entry(row.path.clone()).or_default().push(row);
    }
    let repo = store.repo()?;
    let mut found: BTreeMap<(String, i64, i64), usize> = BTreeMap::new();
    for (path, rows) in &by_path {
        let who: BTreeSet<String> = rows
            .iter()
            .filter_map(|r| events.get(&r.seq))
            .map(Event::who)
            .collect();
        if who.len() < 2 {
            continue;
        }
        let read = |oid: &Option<String>| -> Result<Option<String>> {
            match oid {
                Some(o) => {
                    let data = odb::read_blob(repo, odb::parse_oid(o)?)?;
                    Ok((!is_binary(&data)).then(|| String::from_utf8_lossy(&data).into_owned()))
                }
                None => Ok(Some(String::new())),
            }
        };
        let Some(mut text) = read(&rows[0].before_oid)? else {
            continue;
        };
        let mut origin: Vec<i64> = vec![-1; text::count_lines(&text)];
        for row in rows {
            let Some(new) = read(&row.after_oid)? else {
                break;
            };
            let me = events.get(&row.seq).map(Event::who);
            let mut next = Vec::new();
            for (old_range, new_range, kind) in text::line_ops(&text, &new) {
                if kind == 'e' {
                    next.extend_from_slice(&origin[old_range]);
                    continue;
                }
                for &o in &origin[old_range] {
                    if o > 0 && events.get(&o).map(Event::who) != me {
                        *found.entry((path.clone(), o, row.seq)).or_default() += 1;
                    }
                }
                next.extend(std::iter::repeat_n(row.seq, new_range.len()));
            }
            origin = next;
            text = new;
        }
    }
    let items: Vec<Value> = found
        .iter()
        .map(|((path, first, second), lines)| {
            json!({
                "path": path, "lines": lines,
                "written_by": events.get(first).map(Event::who), "written_in": first,
                "changed_by": events.get(second).map(Event::who), "changed_in": second,
            })
        })
        .collect();
    if out.json {
        out.emit(json!({"since": since, "overlaps": items}));
    } else if items.is_empty() {
        println!("No overlapping edits since {since}.");
    } else {
        for i in &items {
            println!(
                "{}: {} (e:{}) changed {} lines written by {} (e:{})",
                i["path"].as_str().unwrap_or_default(),
                i["changed_by"].as_str().unwrap_or("?"),
                i["changed_in"],
                i["lines"],
                i["written_by"].as_str().unwrap_or("?"),
                i["written_in"]
            );
        }
    }
    Ok(())
}
