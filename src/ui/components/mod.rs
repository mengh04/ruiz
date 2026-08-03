//! Ruiz 跨页面复用的界面构件。

use gpui::{AnyElement, App, SharedString, div, prelude::*, px};
use gpui_component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};

/// 与 gpui-component Story 页面一致的标题栏结构。
pub fn page_header<I>(
    icon: I,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> gpui::Div
where
    I: Into<Icon>,
{
    let colors = cx.theme().colors;

    h_flex()
        .w_full()
        .min_h_20()
        .items_center()
        .justify_between()
        .gap_4()
        .px_6()
        .py_4()
        .border_b_1()
        .border_color(colors.border)
        .child(
            h_flex()
                .min_w_0()
                .items_center()
                .gap_3()
                .child(
                    h_flex()
                        .size_10()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .bg(colors.primary.opacity(0.12))
                        .text_color(colors.primary)
                        .child(Icon::new(icon).size_5()),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_xl()
                                .font_semibold()
                                .text_color(colors.foreground)
                                .child(title.into()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted_foreground)
                                .child(description.into()),
                        ),
                ),
        )
        .when_some(action, |this, action| this.child(action))
}

/// 用于首次使用、空列表与完成状态的统一空状态。
pub fn empty_state<I>(
    icon: I,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> gpui::Div
where
    I: Into<Icon>,
{
    let colors = cx.theme().colors;

    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .gap_3()
        .px_6()
        .py_12()
        .text_center()
        .child(
            h_flex()
                .size_12()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(colors.muted)
                .text_color(colors.muted_foreground)
                .child(Icon::new(icon).size_6()),
        )
        .child(
            v_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .font_semibold()
                        .text_color(colors.foreground)
                        .child(title.into()),
                )
                .child(
                    div()
                        .max_w(px(420.))
                        .text_sm()
                        .text_color(colors.muted_foreground)
                        .child(description.into()),
                ),
        )
        .when_some(action, |this, action| this.child(action))
}
