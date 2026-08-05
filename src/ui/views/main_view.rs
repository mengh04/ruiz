use gpui::{
    AnyView, AppContext as _, Context, Entity, IntoElement, Render, TitlebarOptions, Window,
    WindowBounds, WindowOptions, prelude::*, px, size,
};

use gpui_component::{
    ActiveTheme as _, Icon, IconNamed, Root, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu,
        SidebarMenuItem, SidebarToggleButton,
    },
    v_flex,
};

use crate::assets::RuizIcon;
use crate::ui::components::app_title_bar;

use super::{notes_view::NotesView, review_view::ReviewView, settings::SettingsView};

/// 侧边栏导航的标签页
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Notes,
    Review,
}

pub struct MainView {
    pub active: Tab,
    pub notes: Entity<NotesView>,
    pub review: Entity<ReviewView>,
    sidebar_collapsed: bool,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            active: Tab::Notes,
            notes: cx.new(|cx| NotesView::new(window, cx)),
            review: cx.new(|cx| ReviewView::new(window, cx)),
            sidebar_collapsed: false,
        }
    }

    fn switch(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active = tab;
        if tab == Tab::Review {
            self.review.update(cx, |review, cx| review.load(cx));
        }
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        let bounds = WindowBounds::centered(size(px(960.), px(680.)), cx);
        cx.spawn(async move |_, cx| {
            let result = cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Ruiz 设置".into()),
                        ..TitleBar::title_bar_options()
                    }),
                    window_bounds: Some(bounds),
                    window_min_size: Some(size(px(640.), px(480.))),
                    #[cfg(target_os = "linux")]
                    window_background: gpui::WindowBackgroundAppearance::Transparent,
                    #[cfg(target_os = "linux")]
                    window_decorations: Some(gpui::WindowDecorations::Client),
                    ..Default::default()
                },
                |window, cx| {
                    let settings = cx.new(SettingsView::new);
                    cx.new(|cx| Root::new(settings, window, cx))
                },
            );
            if let Err(error) = result {
                crate::diagnostics::error(
                    "settings.window.open_failed",
                    "Failed to open settings window",
                    serde_json::json!({ "error": error.to_string() }),
                );
                eprintln!("打开设置窗口失败: {error}");
            }
        })
        .detach();
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyView = match self.active {
            Tab::Notes => self.notes.clone().into(),
            Tab::Review => self.review.clone().into(),
        };
        let title_bar = app_title_bar(
            match self.active {
                Tab::Notes => "笔记",
                Tab::Review => "复习",
            },
            cx,
        );

        let view = cx.entity();
        let navigation = SidebarGroup::new("学习空间").child(SidebarMenu::new().children([
            nav_item(
                view.clone(),
                Tab::Notes,
                "笔记",
                RuizIcon::NotebookText,
                self.active == Tab::Notes,
            ),
            nav_item(
                view.clone(),
                Tab::Review,
                "复习",
                RuizIcon::BrainCircuit,
                self.active == Tab::Review,
            ),
        ]));

        let sidebar_collapsed = self.sidebar_collapsed;
        let header = SidebarHeader::new()
            .child(
                h_flex()
                    .size_8()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().primary)
                    .text_color(cx.theme().primary_foreground)
                    .when(!sidebar_collapsed, |this| {
                        this.child(Icon::new(RuizIcon::GraduationCap))
                    })
                    .when(sidebar_collapsed, |this| {
                        this.size_4()
                            .bg(cx.theme().transparent)
                            .text_color(cx.theme().foreground)
                            .child(Icon::new(RuizIcon::GraduationCap))
                    }),
            )
            .when(!sidebar_collapsed, |this| {
                this.child(
                    v_flex().min_w_0().flex_1().gap_0().child("Ruiz").child(
                        v_flex()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("学习与间隔复习"),
                    ),
                )
            });

        let settings_view = view.clone();
        let settings_button = Button::new("open-settings")
            .small()
            .ghost()
            .icon(RuizIcon::Settings)
            .tooltip("设置")
            .on_click(move |_, _, cx| {
                settings_view.update(cx, |this, cx| this.open_settings(cx));
            });

        let toggle_view = view.clone();
        let toggle_button = SidebarToggleButton::new()
            .collapsed(sidebar_collapsed)
            .on_click(move |_, _, cx| {
                toggle_view.update(cx, |this, cx| this.toggle_sidebar(cx));
            });

        let footer = SidebarFooter::new()
            .items_center()
            .when(sidebar_collapsed, |this| {
                this.flex_col().justify_center().gap_1()
            })
            .when(!sidebar_collapsed, |this| {
                this.child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(concat!("Ruiz v", env!("CARGO_PKG_VERSION"))),
                )
            })
            .child(settings_button)
            .child(toggle_button);

        v_flex().size_full().child(title_bar).child(
            h_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    Sidebar::new("main-sidebar")
                        .w(px(208.))
                        .collapsible(SidebarCollapsible::Icon)
                        .collapsed(sidebar_collapsed)
                        .header(header)
                        .child(navigation)
                        .footer(footer),
                )
                .child(
                    v_flex()
                        .id("content")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .child(content),
                ),
        )
    }
}

fn nav_item<I>(
    view: Entity<MainView>,
    tab: Tab,
    label: &'static str,
    icon: I,
    active: bool,
) -> SidebarMenuItem
where
    I: IconNamed,
{
    SidebarMenuItem::new(label)
        .icon(icon)
        .active(active)
        .on_click(move |_, _, cx| {
            view.update(cx, |this, cx| this.switch(tab, cx));
        })
}
