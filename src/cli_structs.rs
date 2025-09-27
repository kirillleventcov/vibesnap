use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(
    name = "vibesnap",
    version = "0.1.0",
    about = "Snapshot-style version control for AI-first coding"
)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Parser, Debug)]
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
        #[clap(long, help = "Show progress bar for large operations")]
        progress: bool,
        #[clap(
            long,
            help = "Snap only specific files (comma-separated)",
            value_delimiter = ','
        )]
        files: Vec<PathBuf>,
        #[clap(long, help = "Snap only the specified file")]
        file: Option<PathBuf>,
    },
    /// List checkpoints
    List {
        #[clap(short, long)]
        track: Option<String>,
        #[clap(long, help = "Show files in each checkpoint as a tree")]
        tree: bool,
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
        #[clap(long, help = "Show progress bar for large operations")]
        progress: bool,
        #[clap(
            long,
            help = "Restore only specific files (comma-separated)",
            value_delimiter = ','
        )]
        files: Vec<PathBuf>,
        #[clap(long, help = "Restore only the specified file")]
        file: Option<PathBuf>,
        #[clap(long, help = "Interactive file selection within checkpoint")]
        interactive_files: bool,
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
    /// Restore the latest checkpoint on the current track
    Latest {
        #[clap(long, help = "Show progress bar for large operations")]
        progress: bool,
    },
    /// Show unified diff between two checkpoints (text files)
    Diff {
        id1: Option<String>,
        id2: Option<String>,
        file: Option<PathBuf>,
        #[clap(long, help = "Show side-by-side diff view")]
        side_by_side: bool,
        #[clap(long, help = "Interactive selection mode")]
        interactive: bool,
    },
    /// Interactive checkpoint selection
    Select {
        #[clap(subcommand)]
        action: SelectCommands,
    },
    /// Show a visual graph of checkpoints and branches
    Graph {
        #[clap(long, help = "Show detailed information for each checkpoint")]
        detailed: bool,
        #[clap(long, help = "Show only specific track")]
        track: Option<String>,
        #[clap(long, help = "Compact view (less spacing)")]
        compact: bool,
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
pub enum SelectCommands {
    /// Interactively restore a checkpoint
    Restore {
        #[clap(long, help = "Show progress bar for large operations")]
        progress: bool,
    },
    /// Interactively switch track
    Switch,
    /// Interactively diff two checkpoints
    Diff {
        #[clap(long, help = "Show side-by-side diff view")]
        side_by_side: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommands {
    /// Show current configuration
    Show,
    /// Edit configuration file
    Edit,
    /// Set a configuration value
    Set { key: String, value: String },
    /// Get a configuration value
    Get { key: String },
    /// Reset configuration to defaults
    Reset {
        #[clap(long)]
        confirm: bool,
    },
    /// Show configuration file location
    Path,
}
