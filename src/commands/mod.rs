pub mod branch;
pub mod config;
pub mod diff;
pub mod init;
pub mod list;
pub mod reset;
pub mod restore;
pub mod snap;
pub mod switch;

use crate::vibe::checkpoint::list_checkpoints;
use crate::vibe::repo::Repo;
use chrono::{Local, TimeZone};

pub fn format_timestamp(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// The newest checkpoints, for error messages that ask for an ID.
pub fn recent_checkpoints_hint() -> String {
    let Ok(repo) = Repo::open() else {
        return String::new();
    };
    let Ok(checkpoints) = list_checkpoints(&repo.conn, None) else {
        return String::new();
    };
    if checkpoints.is_empty() {
        return "No checkpoints yet.".into();
    }
    let mut hint = String::from("Recent checkpoints (newest first):");
    for c in checkpoints.iter().rev().take(10) {
        hint.push_str(&format!(
            "\n  {}  {}  {}",
            c.id,
            format_timestamp(c.timestamp),
            c.note.as_deref().unwrap_or("")
        ));
    }
    hint
}

pub fn tracks_hint() -> String {
    let Ok(repo) = Repo::open() else {
        return String::new();
    };
    let Ok(mut stmt) = repo.conn.prepare("SELECT name FROM tracks ORDER BY name") else {
        return String::new();
    };
    let tracks: Vec<String> = match stmt.query_map([], |row| row.get(0)) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => return String::new(),
    };
    format!("Tracks: {}", tracks.join(", "))
}
