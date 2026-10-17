//! commit, gc, reindex, doctor, install.

use super::out::Out;
use super::record::{begin_write, recent_hook_errors};
use crate::commit;
use crate::error::{AppError, Result, msg};
use crate::paths::resolve_user_path;
use crate::store::index;
use crate::store::ledger;
use crate::store::odb::{self, LABELS_PREFIX, LEDGER_REF};
use crate::store::{Store, lock};
use colored::Colorize;
use serde_json::{Value, json};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn commit(
    out: &Out,
    span: &str,
    message: Option<String>,
    include_mixed: bool,
    exclude: &[PathBuf],
    dry_run: bool,
) -> Result<()> {
    let (store, _lock) = begin_write()?;
    let mut plan = commit::plan(&store, span)?;
    let excluded: Vec<String> = exclude
        .iter()
        .map(|p| resolve_user_path(&store.root, p))
        .collect::<Result<_>>()?;
    plan.paths.retain(|p| !excluded.contains(p));
    plan.mixed.retain(|p| !excluded.contains(p));
    if plan.paths.is_empty() {
        if out.json {
            out.emit(json!({"committed": false, "paths": [], "reason": "nothing agents changed is uncommitted"}));
        } else {
            println!("Nothing to commit: no uncommitted changes by agents in {span}.");
        }
        return Ok(());
    }
    if !plan.mixed.is_empty() && !include_mixed {
        return Err(AppError::Conflict(format!(
            "The user also edited these files during {span}: {}. Pass --include-mixed to commit them anyway, or --exclude them.",
            plan.mixed.join(", ")
        )));
    }
    if dry_run {
        if out.json {
            out.emit(json!({"committed": false, "dry_run": true, "paths": plan.paths, "mixed": plan.mixed}));
        } else {
            println!(
                "Would commit {} files (+{} -{}):",
                plan.paths.len(),
                plan.add,
                plan.del
            );
            for p in &plan.paths {
                println!("  {p}");
            }
        }
        return Ok(());
    }
    let message = match message {
        Some(m) if !m.trim().is_empty() => m,
        _ if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => {
            edit_message(&plan)?
        }
        _ => return Err(AppError::Usage("Pass a commit message with -m".into())),
    };
    let oid = commit::commit(&store, &plan.paths, &message)?;
    commit::mark_committed(&store, plan.end_seq, &oid)?;
    if out.json {
        out.emit(json!({"committed": true, "commit": oid, "paths": plan.paths}));
    } else {
        println!(
            "Committed {} files as {}",
            plan.paths.len(),
            &oid[..oid.len().min(12)]
        );
    }
    Ok(())
}

/// Drafts a message from the diff stat and opens $EDITOR on it.
fn edit_message(plan: &commit::Plan) -> Result<String> {
    let mut draft = format!("Update {} files\n\n", plan.paths.len());
    for p in &plan.paths {
        draft.push_str(&format!("# {p}\n"));
    }
    let path = std::env::temp_dir().join(format!("vibesnap-commit-{}.txt", std::process::id()));
    fs::write(&path, &draft)?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&path)
        .status()?;
    let text = fs::read_to_string(&path).unwrap_or_default();
    let _ = fs::remove_file(&path);
    let message: String = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if !status.success() || message.is_empty() {
        return Err(AppError::Usage("Aborted: empty commit message".into()));
    }
    Ok(message)
}

fn parse_duration(text: &str) -> Option<i64> {
    let (num, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let secs = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 7 * 86400,
        _ => return None,
    };
    Some(n * secs * 1_000_000_000)
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

pub fn gc(out: &Out, keep: Option<String>) -> Result<()> {
    let (mut store, lock) = begin_write()?;
    ledger::catch_up(&mut store, &lock)?;
    let keep_ns = match &keep {
        Some(k) => parse_duration(k)
            .ok_or_else(|| AppError::Usage(format!("Bad duration {k}: use e.g. 7d, 12h")))?,
        None => store.config.retention_days as i64 * 86400 * 1_000_000_000,
    };
    let size_before = dir_size(&store.vibe);
    let cutoff = crate::fsutil::now_ns() - keep_ns;
    let last = index::last_seq(&store.db)?;
    let oldest: i64 = store
        .db
        .query_row("SELECT COALESCE(MIN(seq), 0) FROM events", [], |r| r.get(0))?;
    let first_kept: i64 = store
        .db
        .query_row(
            "SELECT MIN(seq) FROM events WHERE ts_ns >= ?1",
            [cutoff],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(last)
        .min(last);

    let mut pruned = 0;
    if last > 0 && first_kept > oldest {
        // Re-create the kept events as a new chain that starts at first_kept.
        let repo = store.repo()?;
        let kept = index::events_between(&store.db, first_kept - 1, last)?;
        let mut parent: Option<gix::ObjectId> = None;
        let mut rewritten = Vec::new();
        for e in &kept {
            let info = odb::read_commit(repo, odb::parse_oid(&e.commit_oid)?)?;
            let parents: Vec<_> = parent.into_iter().collect();
            let new = odb::write_commit(
                repo,
                info.tree,
                &parents,
                &info.message,
                e.ts_ns / 1_000_000_000,
            )?;
            rewritten.push((e.seq, new));
            parent = Some(new);
        }
        let old_tip = odb::read_ref(repo, LEDGER_REF)?;
        if let Some(new_tip) = parent {
            odb::update_ref(repo, LEDGER_REF, new_tip, old_tip)?;
        }
        let tx = store.db.transaction()?;
        pruned = tx.execute("DELETE FROM events WHERE seq < ?1", [first_kept])?;
        tx.execute("DELETE FROM event_paths WHERE seq < ?1", [first_kept])?;
        tx.execute(
            "DELETE FROM spans WHERE COALESCE(end_seq, start_seq) < ?1",
            [first_kept],
        )?;
        for (seq, oid) in rewritten {
            tx.execute(
                "UPDATE events SET commit_oid = ?1 WHERE seq = ?2",
                rusqlite::params![oid.to_string(), seq],
            )?;
        }
        tx.commit()?;
    }
    store.db.execute(
        "DELETE FROM intents WHERE opened_ns < ?1",
        [crate::fsutil::now_ns() - 86400 * 1_000_000_000],
    )?;
    // VACUUM rewrites the database through the WAL, so truncate after it.
    let _ = store.db.execute_batch("VACUUM;");
    ledger::checkpoint(&store.db);

    let git = Command::new("git")
        .arg("--git-dir")
        .arg(&store.vibe)
        .args(["gc", "--quiet", "--prune=1.hour.ago"])
        .output();
    let repacked = matches!(&git, Ok(o) if o.status.success());
    let size_after = dir_size(&store.vibe);
    drop(lock);
    if out.json {
        out.emit(json!({
            "pruned_events": pruned, "first_kept": first_kept, "repacked": repacked,
            "size_before": size_before, "size_after": size_after,
        }));
    } else {
        println!(
            "Pruned {pruned} events (history now starts at e:{first_kept}). Store: {} -> {}{}",
            crate::text::human_size(size_before as usize),
            crate::text::human_size(size_after as usize),
            if repacked {
                ""
            } else {
                " (git not available: objects left loose)"
            }
        );
    }
    Ok(())
}

pub fn reindex(out: &Out) -> Result<()> {
    let mut store = Store::find()?;
    let _lock = lock::lock(&store.vibe)?;
    ledger::reindex(&mut store)?;
    let n = index::last_seq(&store.db)?;
    if out.json {
        out.emit(json!({"reindexed": true, "last_event": n}));
    } else {
        println!("Rebuilt the index from the ledger (last event e:{n}).");
    }
    Ok(())
}

pub fn doctor(out: &Out) -> Result<()> {
    let store = Store::find()?;
    let repo = store.repo()?;
    let mut checks: Vec<(String, bool, String)> = Vec::new();

    let tip = ledger::tip(&store)?;
    let indexed = index::last_event(&store.db)?;
    let in_sync = tip.map(|(c, _)| c.to_string()) == indexed.as_ref().map(|e| e.commit_oid.clone());
    checks.push((
        "index matches ledger".into(),
        in_sync,
        if in_sync {
            "ok".into()
        } else {
            "the index is behind the ledger; the next write catches up, or run vs reindex".into()
        },
    ));

    let alternates = store.vibe.join("objects/info/alternates");
    if alternates.exists() {
        let target = fs::read_to_string(&alternates).unwrap_or_default();
        let ok = Path::new(target.trim()).is_dir();
        checks.push((
            "borrowing from .git".into(),
            ok,
            if ok {
                target.trim().to_string()
            } else {
                format!("{} is gone", target.trim())
            },
        ));
    }

    let mut missing = Vec::new();
    let mut trees = vec![("ledger".to_string(), tip.map(|(_, t)| t))];
    for (name, commit) in odb::list_refs(repo, LABELS_PREFIX)? {
        trees.push((
            format!("label {name}"),
            odb::read_commit(repo, commit).ok().map(|c| c.tree),
        ));
    }
    for (what, tree) in trees {
        let Some(tree) = tree else { continue };
        match odb::flatten(repo, tree) {
            Ok(files) => {
                let lost = files
                    .values()
                    .filter(|e| !odb::has_object(repo, e.oid))
                    .count();
                if lost > 0 {
                    missing.push(format!("{what}: {lost} files"));
                }
            }
            Err(e) => missing.push(format!("{what}: {e}")),
        }
    }
    checks.push((
        "objects present".into(),
        missing.is_empty(),
        if missing.is_empty() {
            "ok".into()
        } else {
            format!("missing: {} (pruned from .git?)", missing.join("; "))
        },
    ));

    let inbox = fs::read_dir(store.vibe.join("inbox"))
        .map(|d| d.count())
        .unwrap_or(0);
    checks.push(("hook queue".into(), inbox < 100, format!("{inbox} queued")));
    let errors = recent_hook_errors(&store);
    checks.push((
        "hook errors (24h)".into(),
        errors.is_empty(),
        if errors.is_empty() {
            "none".into()
        } else {
            errors.join(" | ")
        },
    ));
    let turns = index::spans(&store.db, "turn")?.len();
    checks.push((
        "hooks installed".into(),
        true,
        if turns > 0 {
            format!("{turns} turns recorded")
        } else {
            "no turns recorded yet (run vs install claude)".into()
        },
    ));

    let problems = checks.iter().filter(|c| !c.1).count();
    if out.json {
        out.emit(json!({
            "ok": problems == 0,
            "checks": checks.iter().map(|(n, ok, d)| json!({"name": n, "ok": ok, "detail": d})).collect::<Vec<_>>(),
        }));
    } else {
        for (name, ok, detail) in &checks {
            println!(
                "{} {name}: {detail}",
                if *ok { "ok  ".green() } else { "FAIL".red() }
            );
        }
    }
    if problems > 0 {
        return Err(msg(format!("doctor found {problems} problem(s)")));
    }
    Ok(())
}

pub const SKILL_MD: &str = include_str!("../../claude-plugin/skills/vibesnap/SKILL.md");

const HOOK_EVENTS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("PreToolUse", Some("Bash|Edit|MultiEdit|Write|NotebookEdit")),
    (
        "PostToolUse",
        Some("Bash|Edit|MultiEdit|Write|NotebookEdit"),
    ),
    (
        "PostToolUseFailure",
        Some("Bash|Edit|MultiEdit|Write|NotebookEdit"),
    ),
    ("SubagentStart", None),
    ("SubagentStop", None),
    ("Stop", None),
];

fn is_ours(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hooks| {
        hooks.iter().any(|h| {
            h["command"]
                .as_str()
                .is_some_and(|c| c.ends_with(" hook claude"))
        })
    })
}

pub fn install(out: &Out, agent: &str, project: bool, uninstall: bool) -> Result<()> {
    if agent != "claude" {
        return Err(AppError::Usage(format!(
            "Unknown agent {agent} (supported: claude)"
        )));
    }
    let base = if project {
        let cwd = std::env::current_dir()?;
        let root = cwd
            .ancestors()
            .find(|d| d.join(".git").exists() || d.join(".vibe").exists())
            .unwrap_or(&cwd)
            .to_path_buf();
        root.join(".claude")
    } else {
        dirs::home_dir()
            .ok_or_else(|| msg("no home directory"))?
            .join(".claude")
    };
    let settings_path = base.join("settings.json");
    let mut settings: Value = match fs::read_to_string(&settings_path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            msg(format!(
                "{} is not valid JSON: {e}",
                settings_path.display()
            ))
        })?,
        Err(_) => json!({}),
    };
    if !settings.is_object() {
        return Err(msg(format!(
            "{} is not a JSON object",
            settings_path.display()
        )));
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    let command = format!("\"{}\" hook claude", exe.display());
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    for (event, matcher) in HOOK_EVENTS {
        let groups = hooks
            .as_object_mut()
            .ok_or_else(|| msg("settings.hooks is not an object"))?
            .entry(*event)
            .or_insert_with(|| json!([]));
        let list = groups
            .as_array_mut()
            .ok_or_else(|| msg(format!("settings.hooks.{event} is not a list")))?;
        list.retain(|g| !is_ours(g));
        if !uninstall {
            let mut group =
                json!({"hooks": [{"type": "command", "command": command, "timeout": 30}]});
            if let Some(m) = matcher {
                group["matcher"] = (*m).into();
            }
            list.push(group);
        }
    }
    fs::create_dir_all(&base)?;
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings)? + "\n",
    )?;
    let skill_dir = base.join("skills").join("vibesnap");
    if uninstall {
        let _ = fs::remove_dir_all(&skill_dir);
    } else {
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), SKILL_MD)?;
    }
    if out.json {
        out.emit(json!({"installed": !uninstall, "settings": settings_path, "skill": skill_dir}));
    } else if uninstall {
        println!(
            "Removed the vibesnap hooks from {} and the skill from {}",
            settings_path.display(),
            skill_dir.display()
        );
    } else {
        println!(
            "Installed hooks into {} and the skill into {}",
            settings_path.display(),
            skill_dir.display()
        );
        println!(
            "New Claude Code sessions in a git repository now record every change (vs status to check)."
        );
    }
    Ok(())
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        msg(e.to_string())
    }
}
