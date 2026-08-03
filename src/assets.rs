use gpui::{App, AssetSource, IntoElement, RenderOnce, Result, SharedString, Window};
use gpui_component::{Icon, IconNamed, icon_named};
use rust_embed::RustEmbed;
use std::{borrow::Cow, collections::HashSet};

/// Ruiz 自己维护的 Lucide 图标集合。
#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct LocalAssets;

/// 优先加载 Ruiz 自有资源，未命中时回退到 gpui-component-assets。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        if let Some(asset) = LocalAssets::get(path) {
            return Ok(Some(asset.data));
        }

        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut seen = HashSet::new();
        let mut assets = Vec::new();

        for asset in LocalAssets::iter().filter(|asset| asset.starts_with(path)) {
            if seen.insert(asset.to_string()) {
                assets.push(asset.into());
            }
        }

        for asset in gpui_component_assets::Assets.list(path)? {
            if seen.insert(asset.to_string()) {
                assets.push(asset);
            }
        }

        Ok(assets)
    }
}

icon_named!(RuizIcon, "assets/icons");

impl RenderOnce for RuizIcon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        Icon::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_icon_overrides_component_asset_with_the_same_path() {
        let source = Assets;
        let local = LocalAssets::get("icons/hard-drive.svg").expect("local override should exist");
        let asset = source
            .load("icons/hard-drive.svg")
            .expect("local icon lookup should not fail")
            .expect("local icon should exist");
        assert_eq!(asset.as_ref(), local.data.as_ref());
    }

    #[test]
    fn falls_back_to_component_assets() {
        let source = Assets;
        let asset = source
            .load("icons/search.svg")
            .expect("fallback icon lookup should not fail")
            .expect("fallback icon should exist");
        let svg = std::str::from_utf8(asset.as_ref()).expect("icon should be valid UTF-8");
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn lists_local_and_fallback_assets_without_duplicates() {
        let source = Assets;
        let assets = source
            .list("icons/")
            .expect("asset listing should not fail");
        assert!(assets.iter().any(|path| path == "icons/notebook-text.svg"));
        assert!(assets.iter().any(|path| path == "icons/search.svg"));
        assert_eq!(
            assets
                .iter()
                .filter(|path| *path == "icons/hard-drive.svg")
                .count(),
            1
        );
    }
}
