mod cli;
mod cli_structs;
mod commands;
mod config;
mod error;
mod vibe;

use clap::Parser;
use colored::*;
use error::Result;

use cli_structs::{Cli, Commands, SelectCommands};

fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.to_string().red());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => commands::init::init_command(&path)?,
        Commands::Snap {
            paths,
            note,
            progress,
            files,
            file,
        } => {
            let selective_files = commands::get_selective_files(files, file);
            commands::snap::snap_command(paths, note, progress, selective_files)?
        }
        Commands::List {
            track,
            tree,
            interactive,
            file,
        } => commands::list::list_checkpoints_command(track, tree, interactive, file)?,
        Commands::Restore {
            checkpoint_id,
            interactive,
            progress,
            files,
            file,
            interactive_files,
        } => {
            let selective_files = commands::get_selective_files(files, file);
            if interactive || checkpoint_id.is_none() {
                cli::interactive::interactive_restore_command(progress, selective_files)?
            } else if interactive_files {
                cli::interactive::interactive_file_restore_command(
                    checkpoint_id.unwrap(),
                    progress,
                )?
            } else {
                commands::restore::restore_command(
                    checkpoint_id.unwrap(),
                    progress,
                    selective_files,
                )?
            }
        }
        Commands::Branch { name, from_id } => commands::branch::branch_command(name, from_id)?,
        Commands::Switch { name, interactive } => {
            if interactive || name.is_none() {
                cli::interactive::interactive_switch_command()?
            } else {
                commands::switch::switch_command(name.unwrap())?
            }
        }
        Commands::Latest { progress } => commands::latest::latest_command(progress)?,
        Commands::Diff {
            id1,
            id2,
            file,
            side_by_side,
            interactive,
        } => {
            if interactive || id1.is_none() || id2.is_none() {
                cli::interactive::interactive_diff_command(file, side_by_side)?
            } else {
                commands::diff::diff_command(id1.unwrap(), id2.unwrap(), file, side_by_side)?
            }
        }
        Commands::Select { action } => match action {
            SelectCommands::Restore { progress } => {
                cli::interactive::interactive_restore_command(progress, None)?
            }
            SelectCommands::Switch => cli::interactive::interactive_switch_command()?,
            SelectCommands::Diff { side_by_side } => {
                cli::interactive::interactive_diff_command(None, side_by_side)?
            }
        },
        Commands::Graph {
            detailed,
            track,
            compact,
        } => commands::graph::graph_command(detailed, track, compact)?,
        Commands::Config { action } => commands::config::config_command(action)?,
        Commands::Reset { confirm } => commands::reset::reset_command(confirm)?,
        Commands::Watch {
            interval,
            stop,
            on_save,
        } => commands::watch::watch_command(interval, stop, on_save)?,
        Commands::Rewind {
            duration,
            to,
            progress,
        } => commands::rewind::rewind_command(duration, to, progress)?,
        Commands::Fastforward { progress } => commands::fastforward::fastforward_command(progress)?,
        Commands::Timeline { track, detailed } => {
            commands::timeline::timeline_command(track, detailed)?
        }
    }
    Ok(())
}
