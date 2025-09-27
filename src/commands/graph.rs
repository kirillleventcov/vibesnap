use crate::error::Result;
use crate::vibe::{db::db_connect, repo::find_repo_root};
use colored::*;

pub fn graph_command(_detailed: bool, track: Option<String>, _compact: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    // This is a placeholder for a more complex graph visualization.
    // For now, it just prints a simplified log.
    println!("{}", "Checkpoint Graph:".bold().cyan());

    let mut query = "SELECT id, track, parent, note, timestamp FROM checkpoints".to_string();
    if let Some(track_name) = track {
        query.push_str(&format!(" WHERE track = '{}'", track_name));
    }
    query.push_str(" ORDER BY timestamp DESC");

    let mut stmt = conn.prepare(&query)?;
    let checkpoints = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    for checkpoint in checkpoints {
        let (id, track, parent, note, _timestamp) = checkpoint?;
        let parent_str = parent.unwrap_or_else(|| "root".to_string());
        println!(
            "* {} ({}) -> {} {}",
            id.green(),
            track.yellow(),
            parent_str.dimmed(),
            note.unwrap_or_default()
        );
    }

    Ok(())
}
