use crate::config::Config;
use crate::error::{AppError, Result};
use crate::vibe::{
    constants::{OBJECTS_DIRNAME, REPO_DIRNAME, SNAPSHOTS_DIRNAME},
    repo::{Repo, set_head},
};
use colored::*;
use std::fs;
use std::path::Path;

pub fn init_command(path: &Path, json: bool) -> Result<()> {
    let config = Config::load();
    let root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let vibe_dir = root.join(REPO_DIRNAME);

    if vibe_dir.exists() {
        return Err(AppError::RepoExists);
    }

    // Create new directory structure
    fs::create_dir_all(vibe_dir.join(OBJECTS_DIRNAME))?;
    fs::create_dir_all(vibe_dir.join(SNAPSHOTS_DIRNAME))?;

    let repo = Repo::open_at(root.clone())?;

    // Use configured default track
    let default_track = config.get_default_track();
    repo.conn.execute(
        "INSERT OR IGNORE INTO tracks(name, head) VALUES (?, ?)",
        (default_track, Option::<String>::None),
    )?;
    set_head(&repo.conn, default_track, None)?;

    let say = |message: &str| {
        if !json {
            println!("{}", message.green());
        }
    };

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
        say("Created .vibeignore with common ignore patterns");
    } else if gitignore_path.exists() {
        say("Using existing .gitignore for ignore patterns");
    } else {
        say("Using existing .vibeignore for ignore patterns");
    }

    if json {
        println!(
            "{}",
            serde_json::json!({"v": 1, "root": root, "track": default_track})
        );
    } else {
        say(&format!(
            "Initialised empty VibeSnap repo in {}",
            vibe_dir.display()
        ));
    }
    Ok(())
}
