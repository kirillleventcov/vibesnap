use crate::config::Config;
use crate::error::Result;

pub fn config_show_command() -> Result<()> {
    let config = Config::load();
    let config_str = toml::to_string_pretty(&config).unwrap();
    println!("{}", config_str);
    Ok(())
}

pub fn config_path_command() -> Result<()> {
    println!("{}", Config::config_path().display());
    Ok(())
}
