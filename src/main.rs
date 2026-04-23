mod cli;
mod cli_structs;
mod commands;
mod config;
mod error;
mod vibe;

use clap::Parser;
use colored::*;
use error::Result;

use cli_structs::{Cli, Commands, ConfigCommands};

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
        Commands::Snap { paths, note, file } => {
            let selective_files = if file.is_empty() { None } else { Some(file) };
            commands::snap::snap_command(paths, note, selective_files)?
        }
        Commands::List {
            track,
            interactive,
            file,
        } => commands::list::list_checkpoints_command(track, interactive, file)?,
        Commands::Restore {
            checkpoint_id,
            interactive,
            file,
        } => {
            let selective_files = if file.is_empty() { None } else { Some(file) };
            if interactive || checkpoint_id.is_none() {
                cli::interactive::interactive_restore_command(selective_files)?
            } else {
                commands::restore::restore_command(checkpoint_id.unwrap(), selective_files)?
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
        Commands::Diff {
            id1,
            id2,
            file,
            interactive,
        } => {
            if interactive || id1.is_none() || id2.is_none() {
                cli::interactive::interactive_diff_command(file)?
            } else {
                commands::diff::diff_command(id1.unwrap(), id2.unwrap(), file)?
            }
        }
        Commands::Config { action } => match action {
            ConfigCommands::Show => commands::config::config_show_command()?,
            ConfigCommands::Path => commands::config::config_path_command()?,
        },
        Commands::Reset { confirm } => commands::reset::reset_command(confirm)?,
    }
    Ok(())
}
