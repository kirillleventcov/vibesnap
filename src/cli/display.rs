use crate::vibe::snapshot::SnapshotManifest;
use chrono::{Local, TimeZone};
use colored::Colorize;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Row, Table};
use console::Style;
use similar::{ChangeTag, TextDiff};
use std::path::Path;

pub fn display_checkpoints_table(
    checkpoints: Vec<(String, String, Option<String>, i64, Option<String>)>,
) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("id"),
            Cell::new("track"),
            Cell::new("parent"),
            Cell::new("when"),
            Cell::new("note"),
        ]);

    for (id, track, parent, timestamp, note) in checkpoints {
        let local_datetime = Local
            .timestamp_opt(timestamp, 0)
            .single()
            .unwrap_or_default();
        let mut row = Row::new();
        row.add_cell(Cell::new(id))
            .add_cell(Cell::new(track))
            .add_cell(Cell::new(parent.unwrap_or_else(|| "-".to_string())))
            .add_cell(Cell::new(
                local_datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            ))
            .add_cell(Cell::new(note.unwrap_or_default()));
        table.add_row(row);
    }

    println!("{}", table);
}

pub fn display_checkpoints_with_tree(
    root: &Path,
    checkpoints: Vec<(String, String, Option<String>, i64, Option<String>)>,
) {
    println!("{}", "📁 VibeSnap Repository".bold().cyan());

    for (id, track, parent, timestamp, note) in checkpoints {
        let local_datetime = Local
            .timestamp_opt(timestamp, 0)
            .single()
            .unwrap_or_default();

        println!(
            "\n{} {} {} ({})",
            "├─".blue(),
            id.green().bold(),
            track.yellow(),
            local_datetime
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
                .dimmed()
        );

        if let Some(note_text) = note {
            if !note_text.is_empty() {
                println!("{}   {}: {}", "│".blue(), "Note".cyan(), note_text);
            }
        }

        if let Some(parent_id) = parent {
            println!(
                "{}   {}: {}",
                "│".blue(),
                "Parent".cyan(),
                parent_id.dimmed()
            );
        }

        if let Ok(manifest) = crate::vibe::snapshot::load_snapshot_manifest(root, &id) {
            println!("{}   {}:", "│".blue(), "Files".cyan());
            display_file_tree(&manifest, "│   ");
        }
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
        let (sign, style) = match change.tag() {
            ChangeTag::Delete => ("-", Style::new().red()),
            ChangeTag::Insert => ("+", Style::new().green()),
            ChangeTag::Equal => (" ", Style::new()),
        };
        println!("{} {}", style.apply_to(sign).bold(), style.apply_to(change));
    }
}

pub fn show_side_by_side_diff(text1: &str, text2: &str, file_name: &str) {
    println!("Diff for {}", file_name.bold());
    let diff = TextDiff::from_lines(text1, text2);

    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            println!("{:-^1$}", "-", 80);
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let (sign, style) = match change.tag() {
                    ChangeTag::Delete => ("-", Style::new().red()),
                    ChangeTag::Insert => ("+", Style::new().green()),
                    ChangeTag::Equal => (" ", Style::new().dim()),
                };
                println!(
                    "{:>4} {} {}",
                    change.old_index().map_or("".to_string(), |i| i.to_string()),
                    style.apply_to(sign).bold(),
                    style.apply_to(change)
                );
            }
        }
    }
}
