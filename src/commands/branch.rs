use crate::error::{AppError, Result};
use crate::vibe::{checkpoint::find_checkpoint, lock::lock_repo, repo::Repo};
use colored::*;

pub fn branch_command(name: String, from_id_opt: Option<String>, json: bool) -> Result<()> {
    let repo = Repo::open()?;
    let _lock = lock_repo(&repo.root)?;

    let mut stmt = repo.conn.prepare("SELECT 1 FROM tracks WHERE name = ?1")?;
    if stmt.exists(rusqlite::params![&name])? {
        return Err(AppError::TrackExists(name));
    }

    let from_checkpoint_id = match from_id_opt {
        Some(id) => Some(find_checkpoint(&repo.conn, &id)?.id),
        None => repo.head()?.checkpoint,
    };

    repo.conn.execute(
        "INSERT INTO tracks(name, head) VALUES (?1, ?2)",
        rusqlite::params![&name, from_checkpoint_id],
    )?;

    if json {
        println!(
            "{}",
            serde_json::json!({"v": 1, "track": name, "head": from_checkpoint_id})
        );
        return Ok(());
    }
    println!(
        "Created track {} at {}",
        name.green(),
        from_checkpoint_id.as_deref().unwrap_or("root").green()
    );

    Ok(())
}
