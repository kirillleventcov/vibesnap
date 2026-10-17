use crate::commands::format_timestamp;
use crate::vibe::checkpoint::Checkpoint;
use crate::vibe::snapshot::SnapshotManifest;
use colored::Colorize;
use similar::{ChangeTag, TextDiff};

pub fn display_compact_checkpoints(
    checkpoints: &[Checkpoint],
    current_checkpoint_id: Option<String>,
) {
    for checkpoint in checkpoints {
        let id = &checkpoint.id;
        let dt = format_timestamp(checkpoint.timestamp);

        let marker = if Some(id) == current_checkpoint_id.as_ref() {
            "●".green()
        } else {
            "◆".cyan()
        };

        let id_display = if Some(id) == current_checkpoint_id.as_ref() {
            id.green().bold()
        } else {
            id.normal()
        };

        let note_display = checkpoint.note.clone().unwrap_or_default();
        println!(
            "{} {} {} - {}",
            marker,
            dt.bright_black(),
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
            // Each change already ends with its newline (except a final line
            // without one), so print! instead of println! to avoid double spacing.
            ChangeTag::Delete => print_line("-".red().bold(), change.to_string().red()),
            ChangeTag::Insert => print_line("+".green().bold(), change.to_string().green()),
            ChangeTag::Equal => print_line(" ".normal(), change.to_string().normal()),
        }
    }
}

fn print_line(sign: colored::ColoredString, line: colored::ColoredString) {
    if line.ends_with('\n') {
        print!("{}{}", sign, line);
    } else {
        println!("{}{}", sign, line);
    }
}
