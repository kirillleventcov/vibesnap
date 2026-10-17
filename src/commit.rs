//! Committing what agents changed, through git so the repo's own hooks run.
//! Used by `vs commit` and by the Stop hook's offer.

use crate::error::{AppError, Result, git_err, msg};
use crate::refs;
use crate::store::Store;
use crate::store::index;
use crate::store::odb::{self, Entry};
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

#[derive(Debug, Default)]
pub struct Plan {
    /// Paths agents changed in the span that differ from git's HEAD now.
    pub paths: Vec<String>,
    /// Of those, the ones the user (or another tool) also changed in the span.
    pub mixed: Vec<String>,
    pub agents: Vec<String>,
    pub add: i64,
    pub del: i64,
    pub end_seq: i64,
}

/// The git HEAD tree of the project (empty when HEAD is unborn).
fn git_head_tree(store: &Store) -> Result<gix::ObjectId> {
    let project = gix::open_opts(&store.root, gix::open::Options::isolated()).map_err(git_err)?;
    match project.rev_parse_single("HEAD") {
        Ok(id) => Ok(id
            .object()
            .map_err(git_err)?
            .peel_to_tree()
            .map_err(git_err)?
            .id),
        Err(_) => Ok(odb::empty_tree()),
    }
}

pub fn plan(store: &Store, span: &str) -> Result<Plan> {
    if store.git().is_none() {
        return Err(AppError::Usage(format!(
            "{} is not the top of a git repository",
            store.root.display()
        )));
    }
    let events = refs::resolve_events(store, span)?;
    let mut agent_paths: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut other_paths: BTreeSet<String> = BTreeSet::new();
    let mut agents = Vec::new();
    for e in &events {
        let rows = index::event_paths(&store.db, e.seq)?;
        if e.actor == "agent" {
            if !agents.contains(&e.who()) {
                agents.push(e.who());
            }
            for r in rows {
                let c = agent_paths.entry(r.path).or_default();
                c.0 += r.added.unwrap_or(0);
                c.1 += r.removed.unwrap_or(0);
            }
        } else if !matches!(e.cause.as_str(), "baseline" | "import") {
            other_paths.extend(rows.into_iter().map(|r| r.path));
        }
    }
    let head = git_head_tree(store)?;
    let current: BTreeMap<String, Entry> = crate::wc::scan::load_cache(store)?
        .into_iter()
        .map(|(p, c)| (p, c.entry))
        .collect();
    let repo = store.repo()?;
    let mut plan = Plan {
        agents,
        end_seq: events.last().map(|e| e.seq).unwrap_or(0),
        ..Default::default()
    };
    for (path, (a, d)) in agent_paths {
        let in_head = odb::entry_at(repo, head, &path)?;
        if in_head == current.get(&path).copied() {
            continue; // committed already, or reverted
        }
        if other_paths.contains(&path) {
            plan.mixed.push(path.clone());
        }
        plan.add += a;
        plan.del += d;
        plan.paths.push(path);
    }
    Ok(plan)
}

fn git(store: &Store, args: &[&str], paths: &[String]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&store.root)
        .args(args)
        .arg("--")
        .args(paths)
        .output()
        .map_err(|e| msg(format!("could not run git: {e}")))?;
    if !output.status.success() {
        return Err(msg(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Stages exactly `paths` and commits only them. Returns the new commit ID.
pub fn commit(store: &Store, paths: &[String], message: &str) -> Result<String> {
    let head = git_head_tree(store)?;
    let repo = store.repo()?;
    // A path that is gone and was never in HEAD has nothing to commit.
    let mut keep = Vec::new();
    for p in paths {
        if store.root.join(p).symlink_metadata().is_ok() || odb::entry_at(repo, head, p)?.is_some()
        {
            keep.push(p.clone());
        }
    }
    if keep.is_empty() {
        return Err(AppError::Usage("Nothing to commit".into()));
    }
    git(store, &["add", "-A"], &keep)?;
    git(store, &["commit", "--quiet", "-m", message], &keep)?;
    Ok(git(store, &["rev-parse", "HEAD"], &[])?.trim().to_string())
}

pub fn mark_committed(store: &Store, end_seq: i64, commit: &str) -> Result<()> {
    store.db.execute(
        "INSERT INTO commit_offers (turn_end_seq, offered_ns, committed_oid) VALUES (?1, ?2, ?3)
         ON CONFLICT(turn_end_seq) DO UPDATE SET committed_oid = excluded.committed_oid",
        rusqlite::params![end_seq, crate::fsutil::now_ns(), commit],
    )?;
    Ok(())
}

/// The Stop hook's one-line offer, or None when there is nothing new to offer.
pub fn offer(store: &Store, turn_number: Option<i64>) -> Result<Option<String>> {
    if store.config.commit_offer == "off" || store.git().is_none() {
        return Ok(None);
    }
    let plan = plan(store, "turn")?;
    if plan.paths.is_empty() {
        return Ok(None);
    }
    let fresh = store.db.execute(
        "INSERT OR IGNORE INTO commit_offers (turn_end_seq, offered_ns) VALUES (?1, ?2)",
        rusqlite::params![plan.end_seq, crate::fsutil::now_ns()],
    )? > 0;
    if !fresh {
        return Ok(None);
    }
    Ok(Some(format!(
        "vibesnap: turn {} changed {} file{} (+{} -{}) by {}. Commit: vs commit turn, or ask Claude.",
        turn_number.unwrap_or_default(),
        plan.paths.len(),
        if plan.paths.len() == 1 { "" } else { "s" },
        plan.add,
        plan.del,
        plan.agents.join(", ")
    )))
}
