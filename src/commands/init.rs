use crate::config::Config;
use crate::error::{AppError, Result};
use crate::vibe::{
    constants::{OBJECTS_DIRNAME, REPO_DIRNAME, SNAPSHOTS_DIRNAME},
    db::{db_connect, ensure_schema},
    repo::write_head,
};
use colored::*;
use std::fs;
use std::path::Path;

pub fn init_command(path: &Path) -> Result<()> {
    let config = Config::load();
    let root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let vibe_dir = root.join(REPO_DIRNAME);

    if vibe_dir.exists() {
        eprintln!("{}", AppError::RepoExists.to_string().red());
        return Err(AppError::RepoExists);
    }

    // Create new directory structure
    fs::create_dir_all(vibe_dir.join(OBJECTS_DIRNAME))?;
    fs::create_dir_all(vibe_dir.join(SNAPSHOTS_DIRNAME))?;

    let conn = db_connect(&root)?;
    ensure_schema(&conn)?;

    // Use configured default track
    let default_track = config.get_default_track();
    conn.execute(
        "INSERT OR IGNORE INTO tracks(name, head) VALUES (?, ?)",
        (default_track, Option::<String>::None),
    )?;

    write_head(&root, default_track, None)?;

    // Only create .vibeignore if .gitignore doesn't exist
    let gitignore_path = root.join(".gitignore");
    let vibeignore_path = root.join(".vibeignore");

    if !gitignore_path.exists() && !vibeignore_path.exists() {
        let vibeignore_content = r#"# VibeSnap ignore patterns
# Patterns in this file will be ignored even if explicitly specified

# Dependencies
node_modules/
vendor/

# Build outputs
target/
dist/
build/
*.o
*.exe
*.dll
*.so

# Logs and temporary files
*.log
*.tmp
*.temp
.DS_Store
Thumbs.db

# IDE and editor files
.vscode/
.idea/
*.swp
*.swo
*~

# VibeSnap directory itself
.vibe/
"#;
        fs::write(vibeignore_path, vibeignore_content)?;
        println!(
            "{}",
            "Created .vibeignore with common ignore patterns".green()
        );
    } else if gitignore_path.exists() {
        println!(
            "{}",
            "Using existing .gitignore for ignore patterns".green()
        );
    } else {
        println!(
            "{}",
            "Using existing .vibeignore for ignore patterns".green()
        );
    }

    println!(
        "{}",
        format!("Initialised empty VibeSnap repo in {}", vibe_dir.display()).green()
    );
    Ok(())
}
