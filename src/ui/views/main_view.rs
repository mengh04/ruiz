use gpui::{AnyView, Context, Entity, IntoElement, Render, Window, prelude::*, px};
use gpui_component::{
    ActiveTheme as _, Icon, IconNamed, h_flex,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu,
        SidebarMenuItem, SidebarToggleButton,
    },
    v_flex,
};

use crate::assets::RuizIcon;

use super::{notes_view::NotesView, review_view::ReviewView, settings::SettingsView};

/// 侧边栏导航的标签页
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Notes,
    Review,
    Settings,
}

pub struct MainView {
    pub active: Tab,
    pub notes: Entity<NotesView>,
    pub review: Entity<ReviewView>,
    pub settings: Entity<SettingsView>,
    sidebar_collapsed: bool,
}

impl MainView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            active: Tab::Notes,
            notes: cx.new(|cx| NotesView::new(window, cx)),
            review: cx.new(|cx| ReviewView::new(window, cx)),
            settings: cx.new(SettingsView::new),
            sidebar_collapsed: false,
        }
    }

    fn switch(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active = tab;
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyView = match self.active {
            Tab::Notes => self.notes.clone().into(),
            Tab::Review => self.review.clone().into(),
            Tab::Settings => self.settings.clone().into(),
        };

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
            nav_item(
                view.clone(),
                Tab::Settings,
                "设置",
                RuizIcon::SlidersHorizontal,
                self.active == Tab::Settings,
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
                    .child(Icon::new(RuizIcon::GraduationCap)),
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

        let toggle_view = view.clone();
        let footer = SidebarFooter::new()
            .when(!sidebar_collapsed, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(concat!("Ruiz v", env!("CARGO_PKG_VERSION"))),
                )
            })
            .child(
                SidebarToggleButton::new()
                    .collapsed(sidebar_collapsed)
                    .on_click(move |_, _, cx| {
                        toggle_view.update(cx, |this, cx| this.toggle_sidebar(cx));
                    }),
            );

        h_flex()
            .size_full()
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
