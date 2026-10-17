use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User configuration from `~/.config/vibesnap/config.toml`. Every field is
/// optional in the file.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    /// `on` (default): the Stop hook tells the user when a turn left uncommitted
    /// agent changes. `off`: never. `ask-agent`: block the stop so Claude asks.
    pub commit_offer: String,
    /// Days of history `vs gc` keeps. Labels are kept until deleted.
    pub retention_days: u64,
    /// Files larger than this are not recorded (and never touched by restore or undo).
    pub max_file_size: u64,
    /// The SessionStart hook creates a store only at a git top level with at most
    /// this many files.
    pub auto_init_max_files: usize,
    /// Whether the SessionStart hook may create a store at a git top level.
    pub auto_init: bool,
    /// Extra patterns never recorded, on top of `.env*`, `*.pem` and `*.key`.
    pub secret_patterns: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            commit_offer: "on".into(),
            retention_days: 7,
            max_file_size: 5 * 1024 * 1024,
            auto_init_max_files: 100_000,
            auto_init: true,
            secret_patterns: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vibesnap")
            .join("config.toml")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|content| toml::from_str(&content).ok())
            .unwrap_or_default()
    }
}
