//! Agent hooks: `vs hook <agent>` reads one hook payload on stdin and records
//! what happened. It always exits 0, so a broken recorder never blocks the
//! agent; errors go to `.vibe/hook.log` and show up in `vs status`.
//!
//! Hooks never wait long for the writer lock. If a restore or gc holds it, the
//! payload is queued in `.vibe/inbox/` and the next writer applies it.

pub mod claude;

use crate::error::{Result, msg};
use crate::fsutil::{now_ns, tmp_path};
use crate::store::Store;
use crate::store::lock::WriteLock;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};

pub fn run(agent: &str) -> i32 {
    let mut input = String::new();
    if std::io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut input)
        .is_err()
    {
        return 0;
    }
    let result = match agent {
        "claude" => claude::handle(&input),
        other => Err(msg(format!("unknown agent '{other}' (supported: claude)"))),
    };
    match result {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => {}
        Err(e) => eprintln!("vibesnap hook: {e}"),
    }
    0
}

pub fn log_error(store: &Store, event: &str, error: &dyn std::fmt::Display) {
    let line = format!(
        "{} {event}: {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        error.to_string().replace('\n', " ")
    );
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store.hook_log())
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Queues a payload for the next writer.
pub fn enqueue(store: &Store, agent: &str, payload: &Value) -> Result<()> {
    let dir = store.vibe.join("inbox");
    fs::create_dir_all(&dir)?;
    let body = serde_json::json!({"agent": agent, "payload": payload}).to_string();
    // Name by time so draining keeps the order; write then rename so a reader
    // never sees half a file.
    let tmp = tmp_path(&dir, ".tmp-");
    fs::write(&tmp, body)?;
    let name = format!(
        "{:020}-{}.json",
        now_ns(),
        tmp.file_name().unwrap_or_default().to_string_lossy()
    );
    fs::rename(&tmp, dir.join(name))?;
    Ok(())
}

/// Applies queued payloads in order. Called by every writer after taking the lock.
pub fn drain_inbox(store: &mut Store, lock: &WriteLock) -> Result<()> {
    let dir = store.vibe.join("inbox");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(());
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for path in files {
        let parsed: Option<Value> = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        let _ = fs::remove_file(&path);
        let Some(item) = parsed else { continue };
        if item["agent"] == "claude" {
            let event = item["payload"]["hook_event_name"]
                .as_str()
                .unwrap_or("?")
                .to_string();
            if let Err(e) = claude::apply(store, Some(lock), &item["payload"], true) {
                log_error(store, &format!("{event} (queued)"), &e);
            }
        }
    }
    Ok(())
}
