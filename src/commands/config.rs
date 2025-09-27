use crate::config::Config;
use crate::error::Result;
use colored::*;

pub fn config_command(action: crate::cli_structs::ConfigCommands) -> Result<()> {
    match action {
        crate::cli_structs::ConfigCommands::Show => {
            let config = Config::load();
            let config_str = toml::to_string_pretty(&config).unwrap();
            println!("{}", config_str);
        }
        crate::cli_structs::ConfigCommands::Edit => {
            let path = Config::config_path();
            if !path.exists() {
                let config = Config::default();
                config.save()?;
            }
            println!("Opening config file: {}", path.display());
            std::process::Command::new("open")
                .arg(path)
                .status()
                .expect("Failed to open config file");
        }
        crate::cli_structs::ConfigCommands::Set { key, value } => {
            let mut config = Config::load();
            // This is a simplified way to set config. A real implementation would handle nested keys.
            config
                .extra
                .insert(key.clone(), toml::Value::String(value.clone()));
            config.save()?;
            println!("Set {} = {}", key, value);
        }
        crate::cli_structs::ConfigCommands::Get { key } => {
            let config = Config::load();
            if let Some(value) = config.extra.get(&key) {
                println!("{}", value);
            } else {
                println!("Key not found");
            }
        }
        crate::cli_structs::ConfigCommands::Reset { confirm } => {
            if confirm
                || dialoguer::Confirm::new()
                    .with_prompt("Reset config to defaults?")
                    .interact()?
            {
                let config = Config::default();
                config.save()?;
                println!("{}", "Config reset to defaults.".green());
            } else {
                println!("Reset cancelled.");
            }
        }
        crate::cli_structs::ConfigCommands::Path => {
            println!("{}", Config::config_path().display());
        }
    }
    Ok(())
}
