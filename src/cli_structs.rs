use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(
    name = "vibesnap",
    version,
    about = "Snapshot-style version control for AI-first coding"
)]
pub struct Cli {
    /// Print machine-readable JSON (schema version "v": 1)
    #[clap(long, global = true)]
    pub json: bool,
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new VibeSnap repo
    Init {
        #[clap(default_value = ".")]
        path: PathBuf,
    },
    /// Create a checkpoint. With paths, only those are re-read and the rest of the
    /// tree keeps its state from HEAD
    Snap {
        /// Files or directories, relative to the current directory. Default: the whole repo
        paths: Vec<PathBuf>,
        #[clap(short, long, default_value = "")]
        note: String,
        #[clap(
            long,
            help = "Snap only specific files (repeatable; same as passing paths)",
            num_args = 1..
        )]
        file: Vec<PathBuf>,
    },
    /// List checkpoints
    List {
        #[clap(short, long)]
        track: Option<String>,
        #[clap(short, long, help = "Interactive selection mode (needs a terminal)")]
        interactive: bool,
        #[clap(long, help = "Show only checkpoints containing this file")]
        file: Option<PathBuf>,
    },
    /// Restore a checkpoint into the working tree. The current state is saved first
    Restore {
        checkpoint_id: Option<String>,
        #[clap(short, long, help = "Interactive selection mode (needs a terminal)")]
        interactive: bool,
        #[clap(
            long,
            help = "Restore only specific files (repeatable)",
            num_args = 1..
        )]
        file: Vec<PathBuf>,
    },
    /// Create a new track
    Branch {
        name: String,
        #[clap(long)]
        from_id: Option<String>,
    },
    /// Switch to another track and sync files
    Switch {
        name: Option<String>,
        #[clap(short, long, help = "Interactive selection mode (needs a terminal)")]
        interactive: bool,
    },
    /// Show unified diff between two checkpoints (text files)
    Diff {
        id1: Option<String>,
        id2: Option<String>,
        file: Option<PathBuf>,
        #[clap(short, long, help = "Interactive selection mode (needs a terminal)")]
        interactive: bool,
    },
    /// Manage configuration settings
    Config {
        #[clap(subcommand)]
        action: ConfigCommands,
    },
    /// Irreversibly delete the .vibe repo and all snaps
    Reset {
        #[clap(
            long,
            help = "Skips the confirmation prompt (required without a terminal)"
        )]
        confirm: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommands {
    /// Show current configuration
    Show,
    /// Show configuration file location
    Path,
}
