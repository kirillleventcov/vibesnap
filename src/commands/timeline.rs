use crate::error::Result;
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, read_head},
};
use colored::*;
use comfy_table::{Cell, Table, presets::UTF8_FULL};

pub fn timeline_command(track: Option<String>, detailed: bool) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;
    let (current_track, current_checkpoint_id) = read_head(&root)?;

    let track_filter = track.unwrap_or(current_track.clone());

    let mut stmt = conn.prepare(
        "SELECT id, timestamp, note, is_auto
         FROM checkpoints
         WHERE track = ?
         ORDER BY timestamp ASC",
    )?;

    let checkpoints: Vec<(String, i64, String, i64)> = stmt
        .query_map(rusqlite::params![track_filter], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if checkpoints.is_empty() {
        println!(
            "{}",
            format!("No checkpoints found on track '{}'", track_filter).yellow()
        );
        return Ok(());
    }

    println!(
        "\n{}",
        format!("Timeline for track: {}", track_filter)
            .cyan()
            .bold()
    );
    println!("{}\n", "━".repeat(60).bright_black());

    if detailed {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);
        table.set_header(vec!["Time", "ID", "Type", "Note"]);

        for (id, timestamp, note, is_auto) in checkpoints {
            let dt = chrono::DateTime::from_timestamp(timestamp, 0)
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S");

            let checkpoint_type = if is_auto == 1 { "auto" } else { "manual" };
            let marker = if Some(&id) == current_checkpoint_id.as_ref() {
                "→"
            } else {
                " "
            };

            let id_cell = if Some(&id) == current_checkpoint_id.as_ref() {
                Cell::new(format!("{} {}", marker, id)).fg(comfy_table::Color::Green)
            } else {
                Cell::new(format!("  {}", id))
            };

            let type_cell = if is_auto == 1 {
                Cell::new(checkpoint_type).fg(comfy_table::Color::Grey)
            } else {
                Cell::new(checkpoint_type).fg(comfy_table::Color::Cyan)
            };

            table.add_row(vec![
                Cell::new(dt.to_string()),
                id_cell,
                type_cell,
                Cell::new(note),
            ]);
        }

        println!("{}", table);
    } else {
        // Compact timeline view
        for (id, timestamp, note, is_auto) in checkpoints {
            let dt = chrono::DateTime::from_timestamp(timestamp, 0)
                .unwrap()
                .format("%H:%M:%S");

            let marker = if Some(&id) == current_checkpoint_id.as_ref() {
                "●".green()
            } else if is_auto == 1 {
                "○".bright_black()
            } else {
                "◆".cyan()
            };

            let time_display = format!("{}", dt).bright_black();
            let id_display = if Some(&id) == current_checkpoint_id.as_ref() {
                id.green().bold()
            } else {
                id.normal()
            };

            println!(
                "{} {} {} - {}",
                marker,
                time_display,
                id_display,
                note.bright_black()
            );
        }
    }

    println!();
    println!("{}", "Legend:".bright_black());
    println!(
        "  {} Manual snap  {} Auto-snap  {} Current position",
        "◆".cyan(),
        "○".bright_black(),
        "●".green()
    );
    println!();

    Ok(())
}
