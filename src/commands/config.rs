use crate::config::Config;
use crate::error::Result;

pub fn config_show_command(json: bool) -> Result<()> {
    let config = Config::load();
    if json {
        let mut out = serde_json::to_value(&config).unwrap_or_default();
        out["v"] = 1.into();
        println!("{out}");
        return Ok(());
    }
    let config_str = toml::to_string_pretty(&config).unwrap();
    println!("{}", config_str);
    Ok(())
}

pub fn config_path_command(json: bool) -> Result<()> {
    let path = Config::config_path();
    if json {
        println!("{}", serde_json::json!({"v": 1, "path": path}));
        return Ok(());
    }
    println!("{}", path.display());
    Ok(())
}
