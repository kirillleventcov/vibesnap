use crate::error::{AppError, Result};
use crate::vibe::{
    db::db_connect,
    repo::{find_repo_root, read_head},
};
use chrono::TimeZone;
use colored::*;

pub fn rewind_command(
    duration: Option<String>,
    to_time: Option<String>,
    progress: bool,
) -> Result<()> {
    let root = find_repo_root(None)?;
    let conn = db_connect(&root)?;
    let (current_track, _) = read_head(&root)?;

    let target_timestamp = if let Some(time_str) = to_time {
        parse_time_today(&time_str)?
    } else if let Some(dur_str) = duration {
        let now = chrono::Utc::now().timestamp();
        let seconds_back = parse_duration(&dur_str)?;
        now - seconds_back
    } else {
        return Err(AppError::Generic(
            "Must provide either --duration or --to".to_string(),
        ));
    };

    // Find the closest checkpoint before the target time on the current track
    let checkpoint_id: String = conn
        .query_row(
            "SELECT id FROM checkpoints
             WHERE track = ? AND timestamp <= ?
             ORDER BY timestamp DESC LIMIT 1",
            rusqlite::params![current_track, target_timestamp],
            |row| row.get(0),
        )
        .map_err(|_| {
            AppError::Generic(format!(
                "No checkpoint found before {}",
                chrono::DateTime::from_timestamp(target_timestamp, 0)
                    .unwrap()
                    .format("%Y-%m-%d %H:%M:%S")
            ))
        })?;

    println!(
        "{}",
        format!("Rewinding to checkpoint {} ...", checkpoint_id).cyan()
    );

    crate::commands::restore::restore_command(checkpoint_id, progress, None)?;

    Ok(())
}

fn parse_duration(s: &str) -> Result<i64> {
    let s = s.trim().to_lowercase();

    // Try to parse formats like "30m", "2h", "1h30m", "45s"
    let mut total_seconds = 0i64;
    let mut current_num = String::new();

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else {
            if current_num.is_empty() {
                return Err(AppError::Generic(format!("Invalid duration format: {}", s)));
            }

            let num: i64 = current_num.parse().map_err(|_| {
                AppError::Generic(format!("Invalid number in duration: {}", current_num))
            })?;

            let multiplier = match ch {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                _ => return Err(AppError::Generic(format!("Invalid time unit: {}", ch))),
            };

            total_seconds += num * multiplier;
            current_num.clear();
        }
    }

    if total_seconds == 0 {
        return Err(AppError::Generic(format!("Invalid duration: {}", s)));
    }

    Ok(total_seconds)
}

fn parse_time_today(time_str: &str) -> Result<i64> {
    let today = chrono::Local::now().date_naive();

    // Parse time in format HH:MM or HH:MM:SS
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(AppError::Generic(format!(
            "Invalid time format: {}. Use HH:MM or HH:MM:SS",
            time_str
        )));
    }

    let hour: u32 = parts[0]
        .parse()
        .map_err(|_| AppError::Generic(format!("Invalid hour: {}", parts[0])))?;

    let minute: u32 = parts[1]
        .parse()
        .map_err(|_| AppError::Generic(format!("Invalid minute: {}", parts[1])))?;

    let second: u32 = if parts.len() == 3 {
        parts[2]
            .parse()
            .map_err(|_| AppError::Generic(format!("Invalid second: {}", parts[2])))?
    } else {
        0
    };

    let naive_time = chrono::NaiveTime::from_hms_opt(hour, minute, second)
        .ok_or_else(|| AppError::Generic(format!("Invalid time: {}", time_str)))?;

    let naive_datetime = chrono::NaiveDateTime::new(today, naive_time);
    let local_datetime = chrono::Local
        .from_local_datetime(&naive_datetime)
        .single()
        .ok_or_else(|| AppError::Generic("Ambiguous local time".to_string()))?;

    Ok(local_datetime.timestamp())
}
