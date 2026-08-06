use gpui::{App, Global};
use serde::{Deserialize, Serialize};

use crate::ai::client::{DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL};

#[derive(Debug, Default, Serialize, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ai: AiConfig,
    pub review: ReviewConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub api_key: Option<String>,
    pub model: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: DEEPSEEK_FLASH_MODEL.into(),
        }
    }
}

impl AiConfig {
    fn normalize(&mut self) {
        self.api_key = self
            .api_key
            .take()
            .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));
        if self.model != DEEPSEEK_FLASH_MODEL && self.model != DEEPSEEK_PRO_MODEL {
            self.model = DEEPSEEK_FLASH_MODEL.into();
        }
    }
}

#[derive(Debug, Default, Serialize, Clone, Deserialize)]
#[serde(default)]
pub struct ReviewConfig {
    /// 启用后按知识单元熟练度生成选择、填空和应用题等动态题型。
    /// 关闭时统一生成自由输入的简答题。
    pub adaptive_answer_formats: bool,
}

pub fn load_config() -> Config {
    let path = config_path();
    let mut config = match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    };
    config.ai.normalize();
    config
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

#[cfg(test)]
mod tests {
    use super::{Config, DEEPSEEK_FLASH_MODEL};

    #[test]
    fn old_openai_compatible_config_migrates_to_deepseek() {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "ai": {
                "api_base": "https://example.invalid/v1",
                "api_key": " secret ",
                "model": "gpt-example"
            }
        }))
        .unwrap();
        config.ai.normalize();
        assert_eq!(config.ai.api_key.as_deref(), Some("secret"));
        assert_eq!(config.ai.model, DEEPSEEK_FLASH_MODEL);
        assert!(!config.review.adaptive_answer_formats);
        assert!(serde_json::to_value(config).unwrap()["ai"]["api_base"].is_null());
    }
}
