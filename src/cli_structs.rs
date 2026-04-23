use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(
    name = "vibesnap",
    version = "1.0.0",
    about = "Snapshot-style version control for AI-first coding"
)]
pub struct Cli {
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
    /// Create a checkpoint from the given paths
    Snap {
        #[clap(default_value = ".")]
        paths: Vec<PathBuf>,
        #[clap(short, long, default_value = "")]
        note: String,
        #[clap(
            long,
            help = "Snap only specific files (repeatable)",
            num_args = 1..
        )]
        file: Vec<PathBuf>,
    },
    /// List checkpoints
    List {
        #[clap(short, long)]
        track: Option<String>,
        #[clap(long, help = "Interactive selection mode")]
        interactive: bool,
        #[clap(long, help = "Show only checkpoints containing this file")]
        file: Option<PathBuf>,
    },
    /// Restore a checkpoint into the working tree (detached)
    Restore {
        checkpoint_id: Option<String>,
        #[clap(long, help = "Interactive selection mode")]
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
        #[clap(long, help = "Interactive selection mode")]
        interactive: bool,
    },
    /// Show unified diff between two checkpoints (text files)
    Diff {
        id1: Option<String>,
        id2: Option<String>,
        file: Option<PathBuf>,
        #[clap(long, help = "Interactive selection mode")]
        interactive: bool,
    },
    /// Manage configuration settings
    Config {
        #[clap(subcommand)]
        action: ConfigCommands,
    },
    /// Irreversibly delete the .vibe repo and all snaps
    Reset {
        #[clap(long, help = "Skips the confirmation prompt")]
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
