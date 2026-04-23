use crate::vibe::snapshot::SnapshotManifest;
use chrono::{Local, TimeZone};
use colored::Colorize;
use similar::{ChangeTag, TextDiff};

pub fn display_compact_checkpoints(
    checkpoints: Vec<(String, String, Option<String>, i64, Option<String>)>,
    current_checkpoint_id: Option<String>,
) {
    for (id, _track, _parent, timestamp, note) in checkpoints {
        let dt = Local
            .timestamp_opt(timestamp, 0)
            .single()
            .unwrap_or_default()
            .format("%Y-%m-%d %H:%M:%S");

        let marker = if Some(&id) == current_checkpoint_id.as_ref() {
            "●".green()
        } else {
            "◆".cyan()
        };

        let id_display = if Some(&id) == current_checkpoint_id.as_ref() {
            id.green().bold()
        } else {
            id.normal()
        };

        let note_display = note.unwrap_or_default();
        println!(
            "{} {} {} - {}",
            marker,
            dt.to_string().bright_black(),
            id_display,
            note_display.bright_black()
        );
    }
}

pub fn display_file_tree(manifest: &SnapshotManifest, prefix: &str) {
    let mut files: Vec<_> = manifest.files.keys().collect();
    files.sort();

    for (i, file_path) in files.iter().enumerate() {
        let is_last = i == files.len() - 1;
        let connector = if is_last { "└─" } else { "├─" };
        println!("{}{} {}", prefix, connector.blue(), file_path);
    }
}

pub fn show_unified_diff(text1: &str, text2: &str, file_name: &str) {
    let diff = TextDiff::from_lines(text1, text2);
    println!("--- a/{}", file_name.red());
    println!("+++ b/{}", file_name.green());
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                println!("{} {}", "-".red().bold(), change.to_string().red())
            }
            ChangeTag::Insert => {
                println!("{} {}", "+".green().bold(), change.to_string().green())
            }
            ChangeTag::Equal => println!("  {}", change),
        }
    }
}
