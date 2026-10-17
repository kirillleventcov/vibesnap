mod cli;
mod cli_structs;
mod commands;
mod config;
mod error;
mod vibe;

use clap::Parser;
use colored::*;
use error::{AppError, Result};
use std::io::IsTerminal;

use cli_structs::{Cli, Commands, ConfigCommands};

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if json || !std::io::stdout().is_terminal() {
        colored::control::set_override(false);
    }
    if let Err(e) = run(cli) {
        if json {
            let payload = serde_json::json!({
                "v": 1,
                "error": { "kind": e.kind(), "message": e.to_string() },
            });
            eprintln!("{payload}");
        } else {
            eprintln!("{}", e.to_string().red());
        }
        std::process::exit(e.exit_code());
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    match cli.command {
        Commands::Init { path } => commands::init::init_command(&path, json)?,
        Commands::Snap { paths, note, file } => {
            let mut paths = paths;
            paths.extend(file);
            commands::snap::snap_command(paths, note, json)?
        }
        Commands::List {
            track,
            interactive,
            file,
        } => {
            if interactive {
                require_terminal()?;
            }
            commands::list::list_checkpoints_command(track, interactive, file, json)?
        }
        Commands::Restore {
            checkpoint_id,
            interactive,
            file,
        } => match checkpoint_id {
            Some(id) if !interactive => commands::restore::restore_command(id, file, json)?,
            _ if interactive => {
                require_terminal()?;
                cli::interactive::interactive_restore_command(file)?
            }
            _ => {
                return Err(AppError::Usage(format!(
                    "Missing checkpoint ID. Pass one, or -i to pick from a menu.\n{}",
                    commands::recent_checkpoints_hint()
                )));
            }
        },
        Commands::Branch { name, from_id } => {
            commands::branch::branch_command(name, from_id, json)?
        }
        Commands::Switch { name, interactive } => match name {
            Some(name) if !interactive => commands::switch::switch_command(name, json)?,
            _ if interactive => {
                require_terminal()?;
                cli::interactive::interactive_switch_command()?
            }
            _ => {
                return Err(AppError::Usage(format!(
                    "Missing track name. Pass one, or -i to pick from a menu.\n{}",
                    commands::tracks_hint()
                )));
            }
        },
        Commands::Diff {
            id1,
            id2,
            file,
            interactive,
        } => match (id1, id2) {
            (Some(id1), Some(id2)) if !interactive => {
                commands::diff::diff_command(id1, id2, file, json)?
            }
            _ if interactive => {
                require_terminal()?;
                cli::interactive::interactive_diff_command(file)?
            }
            _ => {
                return Err(AppError::Usage(format!(
                    "Diff needs two checkpoint IDs. Pass them, or -i to pick from a menu.\n{}",
                    commands::recent_checkpoints_hint()
                )));
            }
        },
        Commands::Config { action } => match action {
            ConfigCommands::Show => commands::config::config_show_command(json)?,
            ConfigCommands::Path => commands::config::config_path_command(json)?,
        },
        Commands::Reset { confirm } => commands::reset::reset_command(confirm, json)?,
    }
    Ok(())
}

pub fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn require_terminal() -> Result<()> {
    if is_interactive_terminal() {
        Ok(())
    } else {
        Err(AppError::Usage(
            "-i/--interactive needs a terminal; pass the arguments instead".into(),
        ))
    }
}
