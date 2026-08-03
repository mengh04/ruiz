use gpui::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Clone, Deserialize)]
pub struct Config {
    pub ai: AiConfig,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

pub fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save_config(config: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(config).unwrap())?;
    Ok(())
}

fn config_path() -> std::path::PathBuf {
    directories::ProjectDirs::from("", "", "ruiz")
        .expect("no home directory")
        .config_dir()
        .join("config.json")
}

#[derive(Debug, Default, Clone)]
pub struct AppSettings {
    pub settings: Config,
}

impl Global for AppSettings {}

impl AppSettings {
    pub fn init(cx: &mut App) {
        let config = load_config();
        cx.set_global(Self { settings: config });
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<AppSettings>()
    }

    pub fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<AppSettings>()
    }
}
