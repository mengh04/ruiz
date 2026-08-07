use std::io::Write as _;

use gpui::{App, Global};
use serde::{Deserialize, Serialize};

use crate::ai::client::{DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL};

#[derive(Debug, Default, Serialize, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ai: AiConfig,
    pub review: ReviewConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub api_key: Option<String>,
    pub model: String,
    pub vision: VisionConfig,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: DEEPSEEK_FLASH_MODEL.into(),
            vision: VisionConfig::default(),
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
        self.vision.normalize();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VisionConfig {
    pub api_base: String,
    pub api_key: Option<String>,
    /// Empty means image recognition is disabled.
    pub model: Option<String>,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            api_base: "https://api.openai.com/v1".into(),
            api_key: None,
            model: None,
        }
    }
}

impl VisionConfig {
    fn normalize(&mut self) {
        self.api_base = self.api_base.trim().trim_end_matches('/').to_string();
        if self.api_base.is_empty() {
            self.api_base = Self::default().api_base;
        }
        self.api_key = self
            .api_key
            .take()
            .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));
        self.model = self.model.take().and_then(|value| {
            let value = value.trim().chars().take(120).collect::<String>();
            (!value.is_empty()).then_some(value)
        });
    }

    pub fn enabled(&self) -> bool {
        self.api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
            && self
                .model
                .as_ref()
                .is_some_and(|model| !model.trim().is_empty())
    }
}

#[derive(Debug, Default, Serialize, Clone, Deserialize)]
#[serde(default)]
pub struct ReviewConfig {
    /// 启用后按知识单元熟练度生成选择、填空和应用题等动态题型。
    /// 关闭时统一生成自由输入的简答题。
    pub adaptive_answer_formats: bool,
}

#[derive(Debug, Default, Serialize, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub sidebar_collapsed: bool,
    pub learning_outline_collapsed: bool,
    pub theme: AppTheme,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppTheme {
    #[default]
    DefaultLight,
    DefaultDark,
    AyuLight,
    AyuDark,
    CatppuccinLatte,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    CatppuccinMocha,
    GruvboxLight,
    GruvboxDark,
    TokyoNight,
    SolarizedLight,
    SolarizedDark,
}

impl<'de> Deserialize<'de> for AppTheme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::parse(&value).unwrap_or_default())
    }
}

impl AppTheme {
    pub const ALL: [Self; 13] = [
        Self::DefaultLight,
        Self::DefaultDark,
        Self::AyuLight,
        Self::AyuDark,
        Self::CatppuccinLatte,
        Self::CatppuccinFrappe,
        Self::CatppuccinMacchiato,
        Self::CatppuccinMocha,
        Self::GruvboxLight,
        Self::GruvboxDark,
        Self::TokyoNight,
        Self::SolarizedLight,
        Self::SolarizedDark,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::DefaultLight => "default_light",
            Self::DefaultDark => "default_dark",
            Self::AyuLight => "ayu_light",
            Self::AyuDark => "ayu_dark",
            Self::CatppuccinLatte => "catppuccin_latte",
            Self::CatppuccinFrappe => "catppuccin_frappe",
            Self::CatppuccinMacchiato => "catppuccin_macchiato",
            Self::CatppuccinMocha => "catppuccin_mocha",
            Self::GruvboxLight => "gruvbox_light",
            Self::GruvboxDark => "gruvbox_dark",
            Self::TokyoNight => "tokyo_night",
            Self::SolarizedLight => "solarized_light",
            Self::SolarizedDark => "solarized_dark",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::DefaultLight => "Ruiz 默认 · 浅色",
            Self::DefaultDark => "Ruiz 默认 · 深色",
            Self::AyuLight => "Ayu Light",
            Self::AyuDark => "Ayu Dark",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::CatppuccinFrappe => "Catppuccin Frappe",
            Self::CatppuccinMacchiato => "Catppuccin Macchiato",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::GruvboxLight => "Gruvbox Light",
            Self::GruvboxDark => "Gruvbox Dark",
            Self::TokyoNight => "Tokyo Night",
            Self::SolarizedLight => "Solarized Light",
            Self::SolarizedDark => "Solarized Dark",
        }
    }

    pub const fn registry_name(self) -> &'static str {
        match self {
            Self::DefaultLight => "Default Light",
            Self::DefaultDark => "Default Dark",
            Self::AyuLight => "Ayu Light",
            Self::AyuDark => "Ayu Dark",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::CatppuccinFrappe => "Catppuccin Frappe",
            Self::CatppuccinMacchiato => "Catppuccin Macchiato",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::GruvboxLight => "Gruvbox Light",
            Self::GruvboxDark => "Gruvbox Dark",
            Self::TokyoNight => "Tokyo Night",
            Self::SolarizedLight => "Solarized Light",
            Self::SolarizedDark => "Solarized Dark",
        }
    }

    pub const fn swatches(self) -> [u32; 5] {
        match self {
            Self::DefaultLight => [0xfafafa, 0xe5e7eb, 0x18181b, 0x3b82f6, 0x22c55e],
            Self::DefaultDark => [0x09090b, 0x27272a, 0xfafafa, 0x3b82f6, 0x22c55e],
            Self::AyuLight => [0xfcfcfc, 0xececed, 0x5c6166, 0x55b4d3, 0xf1ad49],
            Self::AyuDark => [0x0d1016, 0x1f2127, 0xb3b1ad, 0x5ac1fe, 0xfeb454],
            Self::CatppuccinLatte => [0xe5e9ef, 0xdce0e8, 0x4c4f69, 0x7287fd, 0x5aa93b],
            Self::CatppuccinFrappe => [0x232634, 0x414559, 0xc6d0f5, 0x8caaee, 0xe78284],
            Self::CatppuccinMacchiato => [0x1e2030, 0x363a4f, 0xcad3f5, 0x8aadf4, 0xed8796],
            Self::CatppuccinMocha => [0x181825, 0x302d41, 0xcdd6f4, 0x89b4fa, 0xf38ba8],
            Self::GruvboxLight => [0xfbf1c7, 0xebdbb2, 0x3c3836, 0xd79921, 0x67a64f],
            Self::GruvboxDark => [0x1d2021, 0x282828, 0xebdbb2, 0xd79921, 0x98971a],
            Self::TokyoNight => [0x1a1b26, 0x292e42, 0xc0caf5, 0x7aa2f7, 0xf7768e],
            Self::SolarizedLight => [0xfdf6e3, 0xeee8d5, 0x586e75, 0x587573, 0x717558],
            Self::SolarizedDark => [0x002b36, 0x073642, 0xfdf6e3, 0x0d667d, 0x0d7d3f],
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.id() == value)
    }
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
    let serialized = serde_json::to_string_pretty(config).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    file.write_all(serialized.as_bytes())?;
    file.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
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
    use super::{AppTheme, Config, DEEPSEEK_FLASH_MODEL};

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
        assert!(!config.ai.vision.enabled());
        assert!(!config.review.adaptive_answer_formats);
        assert!(!config.ui.sidebar_collapsed);
        assert!(!config.ui.learning_outline_collapsed);
        assert_eq!(config.ui.theme, AppTheme::DefaultLight);
        assert!(serde_json::to_value(config).unwrap()["ai"]["api_base"].is_null());
    }

    #[test]
    fn unknown_theme_falls_back_without_resetting_config() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "ai": { "api_key": "secret" },
            "ui": { "theme": "theme_removed_in_a_future_release" }
        }))
        .unwrap();

        assert_eq!(config.ai.api_key.as_deref(), Some("secret"));
        assert_eq!(config.ui.theme, AppTheme::DefaultLight);
    }
}
