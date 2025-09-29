use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_user")]
    user: String,
    #[serde(default = "default_auto_note_format")]
    auto_note_format: String,
    #[serde(default)]
    show_progress: bool,
    #[serde(default = "default_track_name")]
    default_track: String,
    #[serde(default = "default_watch_interval")]
    watch_interval_minutes: u64,
    #[serde(default)]
    watch_enabled: bool,
    #[serde(flatten)]
    pub extra: HashMap<String, toml::Value>,
}

fn default_watch_interval() -> u64 {
    5
}

fn default_user() -> String {
    "anonymous".to_string()
}

fn default_auto_note_format() -> String {
    "Auto-snap by {user} at {timestamp}".to_string()
}

fn default_track_name() -> String {
    "main".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            user: default_user(),
            auto_note_format: default_auto_note_format(),
            show_progress: false,
            default_track: default_track_name(),
            watch_interval_minutes: default_watch_interval(),
            watch_enabled: false,
            extra: HashMap::new(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vibesnap")
            .join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            return Config::default();
        }
        let content = fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).unwrap();
        fs::write(path, content)
    }

    pub fn format_auto_note(&self) -> String {
        self.auto_note_format.replace("{user}", &self.user).replace(
            "{timestamp}",
            &chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        )
    }

    pub fn get_default_track(&self) -> &str {
        &self.default_track
    }

    pub fn should_show_progress(&self, cli_flag: bool) -> bool {
        cli_flag || self.show_progress
    }

    pub fn watch_interval_minutes(&self) -> u64 {
        self.watch_interval_minutes
    }

    pub fn is_watch_enabled(&self) -> bool {
        self.watch_enabled
    }
}
