use gpui::{Context, IntoElement, Render, div, prelude::*, px};
use gpui_component::{
    ActiveTheme as _, Icon,
    group_box::GroupBoxVariant,
    h_flex,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};

use crate::assets::RuizIcon;
use crate::settings::{AppSettings, save_config};
use crate::state::AppState;
use crate::ui::components::app_title_bar;

pub struct SettingsView;

impl SettingsView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self
    }

    fn ai_page() -> SettingPage {
        SettingPage::new("AI 助手")
            .icon(Icon::new(RuizIcon::Sparkles))
            .description("连接兼容 OpenAI API 的服务，为笔记生成内容并辅助复习判断。")
            .default_open(true)
            .resettable(false)
            .groups([
                SettingGroup::new()
                    .title("服务连接")
                    .description("配置 API 服务地址和访问凭据。修改后会自动保存。")
                    .items([
                        SettingItem::new(
                            "API 地址",
                            SettingField::input(
                                |cx| {
                                    AppSettings::global(cx)
                                        .settings
                                        .ai
                                        .api_base
                                        .clone()
                                        .unwrap_or_default()
                                        .into()
                                },
                                |value, cx| {
                                    AppSettings::global_mut(cx).settings.ai.api_base =
                                        optional_value(value.as_ref());
                                    refresh_ai(cx);
                                },
                            ),
                        )
                        .description("例如 https://api.openai.com/v1，也可以填写兼容服务的地址。")
                        .keywords([
                            "api_base",
                            "base url",
                            "endpoint",
                            "接口地址",
                        ]),
                        SettingItem::new(
                            "API 密钥",
                            SettingField::input(
                                |cx| {
                                    AppSettings::global(cx)
                                        .settings
                                        .ai
                                        .api_key
                                        .clone()
                                        .unwrap_or_default()
                                        .into()
                                },
                                |value, cx| {
                                    AppSettings::global_mut(cx).settings.ai.api_key =
                                        optional_value(value.as_ref());
                                    refresh_ai(cx);
                                },
                            ),
                        )
                        .description("用于认证 API 请求，配置仅保存在本机。")
                        .keywords(["api_key", "token", "密钥", "令牌"]),
                    ]),
                SettingGroup::new()
                    .title("模型")
                    .description("选择生成笔记和判断复习结果时使用的模型。")
                    .item(
                        SettingItem::new(
                            "模型名称",
                            SettingField::input(
                                |cx| {
                                    AppSettings::global(cx)
                                        .settings
                                        .ai
                                        .model
                                        .clone()
                                        .unwrap_or_default()
                                        .into()
                                },
                                |value, cx| {
                                    AppSettings::global_mut(cx).settings.ai.model =
                                        optional_value(value.as_ref());
                                    refresh_ai(cx);
                                },
                            ),
                        )
                        .description("填写服务支持的模型标识，例如 gpt-4.1-mini。")
                        .keywords(["model", "模型", "gpt"]),
                    ),
                SettingGroup::new().title("说明").item(SettingItem::render(
                    |_options, _window, cx| {
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_3()
                            .p_3()
                            .rounded_lg()
                            .bg(cx.theme().colors.muted)
                            .text_color(cx.theme().colors.muted_foreground)
                            .child(Icon::new(RuizIcon::CircleHelp).mt_0p5())
                            .child(
                                v_flex()
                                    .gap_1()
                                    .text_sm()
                                    .child("设置会在输入时自动保存，无需额外点击按钮。")
                                    .child(
                                        "留空的字段将使用应用默认值；修改会立即作用于后续请求。",
                                    ),
                            )
                    },
                )),
            ])
    }

    fn about_page() -> SettingPage {
        SettingPage::new("关于 Ruiz")
            .icon(Icon::new(RuizIcon::CircleHelp))
            .description("一款专注于笔记整理与间隔复习的桌面应用。")
            .resettable(false)
            .group(
                SettingGroup::new().title("应用信息").items([
                    SettingItem::render(|_options, _window, cx| {
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex().gap_1().child("Ruiz").child(
                                    v_flex()
                                        .text_sm()
                                        .text_color(cx.theme().colors.muted_foreground)
                                        .child("把记录、理解和复习放进同一个工作流。"),
                                ),
                            )
                            .child(
                                h_flex()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(cx.theme().colors.secondary)
                                    .text_sm()
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                            )
                    })
                    .keywords(["version", "版本", "ruiz"]),
                    SettingItem::render(|_options, _window, cx| {
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_3()
                            .pt_1()
                            .text_sm()
                            .text_color(cx.theme().colors.muted_foreground)
                            .child(Icon::new(RuizIcon::HardDrive).mt_0p5())
                            .child("AI 配置和笔记数据均保存在本机应用数据目录中。")
                    })
                    .keywords(["data", "local", "数据", "本地"]),
                ]),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(app_title_bar("设置", cx)).child(
            div().flex_1().min_h_0().child(
                Settings::new("ruiz-settings")
                    .sidebar_width(px(216.))
                    .sidebar_size_range(px(184.)..px(300.))
                    .with_group_variant(GroupBoxVariant::Outline)
                    .pages([Self::ai_page(), Self::about_page()]),
            ),
        )
    }
}

fn optional_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// 将配置写入磁盘，并刷新运行时 AI 客户端。
fn refresh_ai(cx: &mut gpui::App) {
    let config = AppSettings::global(cx).settings.clone();
    if let Err(error) = save_config(&config) {
        eprintln!("保存设置失败: {error}");
    }
    AppState::global_mut(cx).configure_ai(&config);
}
