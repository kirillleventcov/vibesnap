use crate::config::Config;
use crate::error::Result;
use crate::vibe::{
    db::db_connect,
    ignore::read_ignore_patterns,
    repo::{find_repo_root, read_head, write_head},
    snapshot::{build_snapshot_manifest, save_snapshot_manifest},
    utils::generate_id,
};
use colored::*;
use notify_debouncer_full::{DebounceEventResult, new_debouncer, notify::*};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

pub fn watch_command(interval_minutes: Option<u64>, stop: bool, on_save: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let pid_file = root.join(".vibe").join("watch.pid");

    if stop {
        return stop_watch(&pid_file);
    }

    let config = Config::load();
    let interval = interval_minutes.unwrap_or_else(|| config.watch_interval_minutes());

    // Check if already running
    if pid_file.exists() {
        let pid_str = std::fs::read_to_string(&pid_file)?;
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            // Check if process is still running
            if is_process_running(pid) {
                println!(
                    "{}",
                    format!(
                        "Watch is already running (PID: {}). Use --stop to stop it.",
                        pid
                    )
                    .yellow()
                );
                return Ok(());
            }
        }
    }

    // Write PID to file
    let pid = std::process::id();
    std::fs::write(&pid_file, pid.to_string())?;

    if on_save {
        // File-watching mode
        println!(
            "{}",
            format!(
                "Started watching {} (auto-snap on file save)",
                root.display()
            )
            .green()
        );
        println!("{}", "Press Ctrl+C to stop watching...".cyan());

        watch_on_file_save(&root)?;
    } else {
        // Time-based mode
        println!(
            "{}",
            format!(
                "Started watching {} (auto-snap every {} minutes)",
                root.display(),
                interval
            )
            .green()
        );
        println!("{}", "Press Ctrl+C to stop watching...".cyan());

        watch_on_interval(&root, interval)?;
    }

    Ok(())
}

fn watch_on_interval(root: &PathBuf, interval: u64) -> Result<()> {
    loop {
        thread::sleep(Duration::from_secs(interval * 60));

        match create_auto_checkpoint(root, "⏱  Time-based") {
            Ok(checkpoint_id) => {
                println!(
                    "{}",
                    format!("⏱  auto-snap {} created", checkpoint_id).bright_black()
                );
            }
            Err(e) => {
                eprintln!("{}", format!("Error creating auto-snap: {}", e).red());
            }
        }
    }
}

fn watch_on_file_save(root: &PathBuf) -> Result<()> {
    let (tx, rx) = channel();
    let root_clone = root.clone();

    // Create debouncer with 2 second delay to batch multiple saves
    let mut debouncer = new_debouncer(
        Duration::from_secs(2),
        None,
        move |result: DebounceEventResult| {
            tx.send(result).ok();
        },
    )
    .map_err(|e| crate::error::AppError::Generic(format!("Failed to create watcher: {}", e)))?;

    // Watch the root directory recursively
    debouncer
        .watcher()
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| {
            crate::error::AppError::Generic(format!("Failed to watch directory: {}", e))
        })?;

    println!("{}", "Watching for file changes...".cyan());

    // Load ignore patterns
    let ignore_patterns = read_ignore_patterns(&root)?;

    loop {
        match rx.recv() {
            Ok(Ok(events)) => {
                // Filter out events in .vibe directory and ignored files
                let mut has_relevant_change = false;
                for event in events.iter() {
                    for path in &event.paths {
                        if !path.starts_with(root.join(".vibe")) {
                            // Check if the path matches ignore patterns
                            let should_ignore = ignore_patterns.iter().any(|pattern| {
                                if let Ok(glob) = glob::Pattern::new(pattern) {
                                    if let Ok(rel_path) = path.strip_prefix(&root) {
                                        return glob.matches_path(rel_path);
                                    }
                                }
                                false
                            });

                            if !should_ignore {
                                has_relevant_change = true;
                                break;
                            }
                        }
                    }
                    if has_relevant_change {
                        break;
                    }
                }

                if has_relevant_change {
                    match create_auto_checkpoint(&root_clone, "💾 File save") {
                        Ok(checkpoint_id) => {
                            println!(
                                "{}",
                                format!("💾 auto-snap {} created", checkpoint_id).bright_black()
                            );
                        }
                        Err(e) => {
                            eprintln!("{}", format!("Error creating auto-snap: {}", e).red());
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("{}", format!("Watch error: {:?}", e).red());
            }
            Err(e) => {
                eprintln!("{}", format!("Channel error: {}", e).red());
                break;
            }
        }
    }

    Ok(())
}

fn stop_watch(pid_file: &PathBuf) -> Result<()> {
    if !pid_file.exists() {
        println!("{}", "Watch is not running.".yellow());
        return Ok(());
    }

    let pid_str = std::fs::read_to_string(pid_file)?;
    if let Ok(pid) = pid_str.trim().parse::<i32>() {
        if is_process_running(pid) {
            kill_process(pid)?;
            println!(
                "{}",
                format!("Stopped watch process (PID: {})", pid).green()
            );
        } else {
            println!("{}", "Watch process was not running.".yellow());
        }
    }

    std::fs::remove_file(pid_file)?;
    Ok(())
}

fn create_auto_checkpoint(root: &PathBuf, checkpoint_type: &str) -> Result<String> {
    let (track, parent_id) = read_head(root)?;
    let conn = db_connect(root)?;
    let checkpoint_id = generate_id(&track);

    // Build manifest for entire working directory
    let manifest = build_snapshot_manifest(root, &[PathBuf::from(".")])?;

    // Skip if no files
    if manifest.files.is_empty() {
        return Ok(checkpoint_id);
    }

    save_snapshot_manifest(root, &checkpoint_id, &manifest)?;

    let note = format!(
        "{} at {}",
        checkpoint_type,
        chrono::Local::now().format("%H:%M:%S")
    );

    conn.execute(
        "INSERT INTO checkpoints(id, track, parent, timestamp, note, is_auto) VALUES (?, ?, ?, ?, ?, 1)",
        rusqlite::params![
            checkpoint_id,
            track,
            parent_id,
            chrono::Utc::now().timestamp(),
            note
        ],
    )?;

    conn.execute(
        "UPDATE tracks SET head = ? WHERE name = ?",
        rusqlite::params![checkpoint_id, track],
    )?;

    write_head(root, &track, Some(&checkpoint_id))?;

    Ok(checkpoint_id)
}

#[cfg(unix)]
fn is_process_running(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_running(_pid: i32) -> bool {
    // On non-Unix systems, assume it's running
    true
}

#[cfg(unix)]
fn kill_process(pid: i32) -> Result<()> {
    unsafe {
        if libc::kill(pid, libc::SIGTERM) == 0 {
            Ok(())
        } else {
            Err(crate::error::AppError::Generic(format!(
                "Failed to kill process {}",
                pid
            )))
        }
    }
}

#[cfg(not(unix))]
fn kill_process(_pid: i32) -> Result<()> {
    Err(crate::error::AppError::Generic(
        "Process killing not supported on this platform".to_string(),
    ))
}
