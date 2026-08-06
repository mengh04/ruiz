use gpui::{Context, IntoElement, Render, div, prelude::*, px};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _,
    group_box::GroupBoxVariant,
    h_flex,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
    tag::{Tag, TagVariant},
    v_flex,
};

use crate::ai::client::{DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL};
use crate::assets::RuizIcon;
use crate::settings::{AppSettings, save_config};
use crate::state::AppState;
use crate::ui::components::app_title_bar;

pub struct SettingsView;

impl SettingsView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self
    }

    fn review_page() -> SettingPage {
        SettingPage::new("复习")
            .icon(Icon::new(RuizIcon::BrainCircuit))
            .description("控制动态复习题型和作答方式。")
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("实验性功能")
                    .description("Beta 功能可能仍在调整中，关闭后会回到稳定的简答题流程。")
                    .item(
                        SettingItem::render(|options, _window, cx| {
                            let enabled = AppSettings::global(cx)
                                .settings
                                .review
                                .adaptive_answer_formats;
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_start()
                                .gap_3()
                                .when(options.disabled, |this| this.opacity(0.5))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .flex_1()
                                        .gap_1()
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_2()
                                                .flex_wrap()
                                                .child(div().text_sm().child("根据熟练度选择作答方式"))
                                                .child(
                                                    Tag::new()
                                                        .with_variant(TagVariant::Warning)
                                                        .small()
                                                        .child("Beta"),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().colors.muted_foreground)
                                                .child("开启后会根据熟练度在选择题、填空题、简答题和应用题之间切换。"),
                                        ),
                                )
                                .child(
                                    Switch::new("adaptive-answer-formats")
                                        .checked(enabled)
                                        .disabled(options.disabled)
                                        .on_click(|checked, window, cx| {
                                            AppSettings::global_mut(cx)
                                                .settings
                                                .review
                                                .adaptive_answer_formats = *checked;
                                            save_settings(cx);
                                            window.refresh();
                                        }),
                                )
                        })
                        .keywords(["review", "adaptive", "mastery", "熟练度", "题型", "beta"]),
                    ),
            )
    }

    fn ai_page() -> SettingPage {
        SettingPage::new("AI 助手")
            .icon(Icon::new(RuizIcon::Sparkles))
            .description("连接 DeepSeek API，为材料整理、知识提取、出题和复习判断提供支持。")
            .default_open(true)
            .resettable(false)
            .groups([
                SettingGroup::new()
                    .title("DeepSeek 连接")
                    .description("使用官方接口 https://api.deepseek.com，修改后会自动保存。")
                    .item(
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
                    ),
                SettingGroup::new()
                    .title("模型")
                    .description("选择生成笔记和判断复习结果时使用的模型。")
                    .item(
                        SettingItem::new(
                            "DeepSeek 模型",
                            SettingField::dropdown(
                                vec![
                                    (DEEPSEEK_FLASH_MODEL.into(), "V4 Flash".into()),
                                    (DEEPSEEK_PRO_MODEL.into(), "V4 Pro".into()),
                                ],
                                |cx| AppSettings::global(cx).settings.ai.model.clone().into(),
                                |value, cx| {
                                    AppSettings::global_mut(cx).settings.ai.model =
                                        value.to_string();
                                    refresh_ai(cx);
                                },
                            )
                            .default_value(DEEPSEEK_FLASH_MODEL),
                        )
                        .description("Flash 成本更低、速度更快；Pro 适合更复杂的材料分析。")
                        .keywords(["model", "模型", "deepseek", "flash", "pro"]),
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
                                    .child("所有 DeepSeek 请求统一使用高强度思考模式，并按工作阶段分配输出预算。")
                                    .child("DeepSeek V4 支持 1M 上下文；Ruiz 会按工作阶段控制单次输出预算。"),
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
                    .pages([Self::ai_page(), Self::review_page(), Self::about_page()]),
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
    save_settings(cx);
    let config = AppSettings::global(cx).settings.clone();
    AppState::global_mut(cx).configure_ai(&config);
}

/// 将当前配置写入磁盘。
fn save_settings(cx: &mut gpui::App) {
    let config = AppSettings::global(cx).settings.clone();
    if let Err(error) = save_config(&config) {
        crate::diagnostics::error(
            "settings.save_failed",
            "Failed to save settings",
            serde_json::json!({ "error": format!("{error:#}") }),
        );
        eprintln!("保存设置失败: {error}");
    }
}
