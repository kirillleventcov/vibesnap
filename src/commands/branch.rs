use crate::error::{AppError, Result};
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, read_head},
};
use colored::*;

pub fn branch_command(name: String, from_id_opt: Option<String>) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;

    let mut stmt = conn.prepare("SELECT 1 FROM tracks WHERE name = ?1")?;
    if stmt.exists(rusqlite::params![&name])? {
        return Err(AppError::TrackExists(name));
    }

    let from_checkpoint_id = match from_id_opt {
        Some(id) => Some(id),
        None => read_head(&root)?.1,
    };

    conn.execute(
        "INSERT INTO tracks(name, head) VALUES (?1, ?2)",
        rusqlite::params![&name, from_checkpoint_id],
    )?;

    println!(
        "{}",
        format!(
            "Created track {} at {}",
            name.green(),
            from_checkpoint_id.as_deref().unwrap_or("root").green()
        )
    );

    Ok(())
}
