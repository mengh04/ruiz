use anyhow::{Result, anyhow};
use gpui::App;
use gpui_component::{Theme, ThemeRegistry};

use crate::settings::{AppSettings, AppTheme};

const BUNDLED_THEMES: &str = include_str!("../assets/themes/bundled.json");

pub fn init(cx: &mut App) -> Result<()> {
    ThemeRegistry::global_mut(cx).load_themes_from_str(BUNDLED_THEMES)?;
    apply(AppSettings::global(cx).settings.ui.theme, cx)
}

pub fn apply(selected: AppTheme, cx: &mut App) -> Result<()> {
    let config = ThemeRegistry::global(cx)
        .themes()
        .get(selected.registry_name())
        .cloned()
        .ok_or_else(|| anyhow!("找不到内置主题 {}", selected.registry_name()))?;
    Theme::global_mut(cx).apply_config(&config);
    cx.refresh_windows();
    Ok(())
}

#[cfg(test)]
mod tests {
    use gpui_component::ThemeSet;

    use super::BUNDLED_THEMES;
    use crate::settings::AppTheme;

    #[test]
    fn bundled_theme_names_match_settings() {
        let set: ThemeSet = serde_json::from_str(BUNDLED_THEMES).unwrap();
        let names = set
            .themes
            .iter()
            .map(|theme| theme.name.as_ref())
            .collect::<std::collections::HashSet<_>>();

        for selected in AppTheme::ALL {
            if !selected.registry_name().starts_with("Default ") {
                assert!(names.contains(selected.registry_name()));
            }
        }
    }
}
