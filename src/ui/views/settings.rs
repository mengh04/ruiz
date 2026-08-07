use gpui::{Anchor, Context, IntoElement, Render, div, prelude::*, px, rgb};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    WindowExt as _,
    button::Button,
    group_box::GroupBoxVariant,
    h_flex,
    notification::Notification,
    popover::Popover,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
    tag::{Tag, TagVariant},
    v_flex,
};

use crate::ai::client::{DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL};
use crate::assets::RuizIcon;
use crate::settings::{AppSettings, AppTheme, save_config};
use crate::state::AppState;
use crate::ui::components::app_title_bar;

pub struct SettingsView;

impl SettingsView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self
    }

    fn appearance_page() -> SettingPage {
        SettingPage::new("外观")
            .icon(Icon::new(IconName::Palette))
            .description("选择适合阅读和复习的界面配色。")
            .default_open(true)
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("主题")
                    .description("主题会立即应用到所有 Ruiz 窗口，并在重启后保留。")
                    .items([
                        SettingItem::render(|options, _window, cx| {
                            let saved = AppSettings::global(cx).settings.ui.theme;
                            let active_theme_name = cx.theme().theme_name().clone();
                            let trigger = Button::new("theme-picker-trigger")
                                .label(saved.label())
                                .dropdown_caret(true)
                                .outline()
                                .disabled(options.disabled);
                            Popover::new("theme-picker-popover")
                                .anchor(Anchor::TopRight)
                                .trigger(trigger)
                                .on_open_change(|open, _, cx| {
                                    if !*open {
                                        restore_saved_theme(cx);
                                    }
                                })
                                .content(move |_, _window, cx| {
                                    let popover = cx.entity();
                                    v_flex()
                                        .id("theme-picker-options")
                                        .w(px(280.))
                                        .max_h(px(420.))
                                        .overflow_y_scroll()
                                        .gap_1()
                                        .children(AppTheme::ALL.into_iter().map(|theme| {
                                            let popover = popover.clone();
                                            Button::new(theme.id())
                                                .w_full()
                                                .justify_start()
                                                .label(theme.label())
                                                .selected(
                                                    active_theme_name.as_ref()
                                                        == theme.registry_name(),
                                                )
                                                .on_hover(move |hovered, _, cx| {
                                                    if *hovered {
                                                        preview_theme(theme, cx);
                                                    }
                                                })
                                                .on_click(move |_, window, cx| {
                                                    select_theme(theme, cx);
                                                    popover.update(cx, |state, cx| {
                                                        state.dismiss(window, cx);
                                                    });
                                                })
                                        }))
                                })
                        })
                        .description("内置 Ayu、Catppuccin、Gruvbox、Tokyo Night 和 Solarized。")
                        .keywords([
                            "theme",
                            "appearance",
                            "dark",
                            "light",
                            "主题",
                            "外观",
                            "深色",
                            "浅色",
                        ]),
                        SettingItem::render(|_options, _window, cx| {
                            let selected = AppSettings::global(cx).settings.ui.theme;
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .gap_1()
                                        .child(div().text_sm().child("当前色板"))
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().colors.muted_foreground)
                                                .child(selected.label()),
                                        ),
                                )
                                .child(h_flex().gap_1p5().children(
                                    selected.swatches().into_iter().map(|color| {
                                        div()
                                            .size_5()
                                            .rounded_full()
                                            .border_1()
                                            .border_color(cx.theme().colors.border)
                                            .bg(rgb(color))
                                    }),
                                ))
                        })
                        .keywords(["palette", "colors", "色板", "颜色"]),
                    ]),
            )
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
                    SettingItem::render(|options, _window, cx| {
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .when(options.disabled, |this| this.opacity(0.5))
                            .child(
                                v_flex().min_w_0().gap_1().child("诊断日志").child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().colors.muted_foreground)
                                        .child(
                                            "排查导入、生成或复习异常时，可在这里打开日志目录。",
                                        ),
                                ),
                            )
                            .child(
                                Button::new("open-diagnostics")
                                    .icon(IconName::FolderOpen)
                                    .label("打开日志目录")
                                    .outline()
                                    .disabled(options.disabled)
                                    .on_click(|_, window, cx| {
                                        match crate::diagnostics::open_log_directory() {
                                            Ok(()) => window.push_notification(
                                                Notification::success("已打开诊断日志目录"),
                                                cx,
                                            ),
                                            Err(error) => {
                                                crate::diagnostics::error(
                                                    "diagnostics.open_failed",
                                                    "Failed to open diagnostics directory",
                                                    serde_json::json!({
                                                        "error": format!("{error:#}")
                                                    }),
                                                );
                                                window.push_notification(
                                                    Notification::error(format!(
                                                        "打开日志目录失败: {error}"
                                                    )),
                                                    cx,
                                                );
                                            }
                                        }
                                    }),
                            )
                    })
                    .keywords(["logs", "diagnostics", "日志", "诊断"]),
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
                    .pages([
                        Self::appearance_page(),
                        Self::ai_page(),
                        Self::review_page(),
                        Self::about_page(),
                    ]),
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

fn preview_theme(theme: AppTheme, cx: &mut gpui::App) {
    if let Err(error) = crate::themes::apply(theme, cx) {
        crate::diagnostics::error(
            "theme.preview_failed",
            "Failed to preview theme",
            serde_json::json!({
                "theme": theme.id(),
                "error": format!("{error:#}"),
            }),
        );
    }
}

fn restore_saved_theme(cx: &mut gpui::App) {
    preview_theme(AppSettings::global(cx).settings.ui.theme, cx);
}

fn select_theme(theme: AppTheme, cx: &mut gpui::App) {
    AppSettings::global_mut(cx).settings.ui.theme = theme;
    save_settings(cx);
    preview_theme(theme, cx);
}
