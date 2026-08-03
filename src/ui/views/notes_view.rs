//! 笔记视图：导入学习材料、AI 出题、浏览卡片。

use gpui::{
    AnyWindowHandle, Context, Entity, IntoElement, Render, ScrollHandle, SharedString, Window, div,
    point, prelude::*, px,
};
use gpui_component::Disableable as _;
use gpui_component::alert::Alert;
use gpui_component::breadcrumb::{Breadcrumb, BreadcrumbItem};
use gpui_component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_component::dialog::DialogButtonProps;
use gpui_component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::skeleton::Skeleton;
use gpui_component::theme::ActiveTheme as _;
use gpui_component::{
    Icon, IconName, Placement, Root, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};

use crate::assets::RuizIcon;
use crate::db;
use crate::domain::card::Card;
use crate::domain::note::Note;
use crate::state::AppState;
use crate::ui::components::{empty_state, page_header};

pub struct NotesView {
    notes: Vec<Note>,
    /// 笔记页内部导航状态，实体在切换主 Tab 时不会重建。
    page: NotesPage,
    cards: Vec<Card>,
    title: Entity<InputState>,
    content: Entity<InputState>,
    question_count: Entity<InputState>,
    notes_loading: bool,
    cards_loading: bool,
    saving: bool,
    generating: bool,
    deleting_note_id: Option<i64>,
    message: Option<Message>,
    /// 窗口句柄（清空输入框等窗口操作需要）
    window: AnyWindowHandle,
    /// 笔记列表滚动
    notes_scroll: ScrollHandle,
    /// 详情区滚动
    detail_scroll: ScrollHandle,
}

/// 资料库是一级页面，单篇笔记是二级页面。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NotesPage {
    #[default]
    Library,
    Detail(i64),
}

impl NotesPage {
    fn note_id(self) -> Option<i64> {
        match self {
            Self::Library => None,
            Self::Detail(id) => Some(id),
        }
    }
}

/// 顶部的操作结果提示。
#[derive(Clone)]
enum Message {
    Success(String),
    Error(String),
}

impl NotesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let window_handle = window.window_handle();
        let title =
            cx.new(|cx| InputState::new(window, cx).placeholder("笔记标题，如：计算机网络 第三章"));
        let content = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("把学习材料粘贴到这里，md / 网页文本均可…")
                .multi_line(true)
                .rows(8)
        });
        let question_count = cx.new(|cx| InputState::new(window, cx).placeholder("10"));
        let mut view = Self {
            notes: Vec::new(),
            page: NotesPage::Library,
            cards: Vec::new(),
            title,
            content,
            question_count,
            notes_loading: true,
            cards_loading: false,
            saving: false,
            generating: false,
            deleting_note_id: None,
            message: None,
            window: window_handle,
            notes_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
        };
        view.refresh_notes(cx);
        view
    }

    fn refresh_notes(&mut self, cx: &mut Context<Self>) {
        let pool = AppState::global(cx).pool.clone();
        self.notes_loading = true;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::notes::list(&pool).await
                    })
                    .await;
                    match result {
                        Ok(notes) => {
                            this.update(&mut cx, |this, cx| {
                                this.notes = notes;
                                this.notes_loading = false;
                                // 保持选中项；若被删除则清空
                                if let Some(id) = this.page.note_id()
                                    && !this.notes.iter().any(|n| n.id == id)
                                {
                                    this.page = NotesPage::Library;
                                    this.cards.clear();
                                    this.cards_loading = false;
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.notes_loading = false;
                                this.message = Some(Message::Error(format!("加载笔记失败: {e}")));
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn refresh_cards(&mut self, cx: &mut Context<Self>) {
        let Some(note_id) = self.page.note_id() else {
            self.cards_loading = false;
            return;
        };
        let pool = AppState::global(cx).pool.clone();
        self.cards_loading = true;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::cards::by_note(&pool, note_id).await
                    })
                    .await;
                    match result {
                        Ok(cards) => {
                            this.update(&mut cx, |this, cx| {
                                if this.page.note_id() == Some(note_id) {
                                    this.cards = cards;
                                    this.cards_loading = false;
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                if this.page.note_id() == Some(note_id) {
                                    this.cards_loading = false;
                                    this.message =
                                        Some(Message::Error(format!("加载卡片失败: {e}")));
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn save_note(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let pool = AppState::global(cx).pool.clone();
        let title = self.title.read(cx).value().trim().to_string();
        let content = self.content.read(cx).value().to_string();
        if title.is_empty() || content.trim().is_empty() {
            window.push_notification("标题和内容都不能为空", cx);
            cx.notify();
            return false;
        }
        self.saving = true;
        self.message = None;
        cx.notify();
        // 清空输入框需要 window，提前取出句柄
        let title_input = self.title.clone();
        let content_input = self.content.clone();
        let window_handle = self.window;
        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::notes::create(&pool, &title, &content).await
                    })
                    .await;
                    match result {
                        Ok(id) => {
                            // 清空输入框（InputState::set_value 需要 window）
                            cx.update_window(window_handle, move |_view, window, cx| {
                                title_input.update(cx, |s, cx| s.set_value("", window, cx));
                                content_input.update(cx, |s, cx| s.set_value("", window, cx));
                                window.push_notification("笔记已保存", cx);
                            })
                            .ok();
                            this.update(&mut cx, |this, cx| {
                                this.saving = false;
                                this.page = NotesPage::Detail(id);
                                this.message = Some(Message::Success("笔记已保存".into()));
                                this.refresh_notes(cx);
                                this.refresh_cards(cx);
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.saving = false;
                                this.message = Some(Message::Error(format!("保存笔记失败: {e}")));
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
            },
        )
        .detach();
        true
    }

    fn select_note(&mut self, id: i64, cx: &mut Context<Self>) {
        self.page = NotesPage::Detail(id);
        self.cards.clear();
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        self.refresh_cards(cx);
        cx.notify();
    }

    fn show_library(&mut self, cx: &mut Context<Self>) {
        self.page = NotesPage::Library;
        self.cards_loading = false;
        cx.notify();
    }

    fn open_import_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title.clone();
        let content = self.content.clone();
        let view = cx.entity();
        let sheet_width = px((window.viewport_size().width.as_f32() - 24.).clamp(320., 520.));
        window.open_sheet_at(Placement::Right, cx, move |sheet, _, cx| {
            let save_view = view.clone();
            let cancel_view = view.clone();
            let close_view = view.clone();
            sheet
                .overlay(true)
                .overlay_closable(true)
                .resizable(true)
                .size(sheet_width)
                .on_close(move |_, _, cx| {
                    close_view.update(cx, |_, cx| cx.notify());
                })
                .title(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(RuizIcon::NotebookText).size_4())
                        .child("导入学习材料"),
                )
                .child(
                    v_flex()
                        .size_full()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "粘贴课程笔记、文章或 Markdown，保存后即可让 AI 生成学习卡片。",
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("标题"))
                                .child(Input::new(&title)),
                        )
                        .child(
                            v_flex()
                                .min_h_0()
                                .flex_1()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("正文内容"))
                                .child(Input::new(&content).h(px(320.))),
                        ),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-import")
                                .label("取消")
                                .outline()
                                .on_click(move |_, window, cx| {
                                    window.close_sheet(cx);
                                    cancel_view.update(cx, |_, cx| cx.notify());
                                }),
                        )
                        .child(
                            Button::new("save-note")
                                .icon(IconName::Check)
                                .label("保存笔记")
                                .primary()
                                .on_click(move |_, window, cx| {
                                    let started =
                                        save_view.update(cx, |this, cx| this.save_note(window, cx));
                                    if started {
                                        window.close_sheet(cx);
                                    }
                                }),
                        ),
                )
        });
        cx.notify();
    }

    fn confirm_delete_note(
        &mut self,
        note_id: i64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            dialog
                .title("删除这篇笔记？")
                .description(format!(
                    "“{title}”以及它生成的学习卡片和复习记录都会被删除，此操作无法撤销。"
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| this.delete_note(note_id, cx));
                        true
                    }
                })
                .on_close({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |_, cx| cx.notify());
                    }
                })
        });
        cx.notify();
    }

    fn delete_note(&mut self, note_id: i64, cx: &mut Context<Self>) {
        if self.deleting_note_id.is_some() {
            return;
        }
        let pool = AppState::global(cx).pool.clone();
        self.deleting_note_id = Some(note_id);
        self.message = None;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::notes::delete(&pool, note_id).await
                    })
                    .await;
                    match result {
                        Ok(()) => {
                            this.update(&mut cx, |this, cx| {
                                this.deleting_note_id = None;
                                if this.page.note_id() == Some(note_id) {
                                    this.page = NotesPage::Library;
                                    this.cards.clear();
                                    this.cards_loading = false;
                                }
                                this.message = Some(Message::Success("笔记已删除".into()));
                                this.refresh_notes(cx);
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(error) => {
                            this.update(&mut cx, |this, cx| {
                                this.deleting_note_id = None;
                                this.message =
                                    Some(Message::Error(format!("删除笔记失败: {error}")));
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
            },
        )
        .detach();
    }

    /// AI 出题：把选中笔记拆成若干道题并入库
    fn generate(&mut self, cx: &mut Context<Self>) {
        let Some(note_id) = self.page.note_id() else {
            self.message = Some(Message::Error("请先选中一篇笔记".into()));
            cx.notify();
            return;
        };
        let Some(note) = self.notes.iter().find(|n| n.id == note_id) else {
            return;
        };
        let Some(ai) = AppState::global(cx).ai.clone() else {
            self.message = Some(Message::Error(
                "请先在「设置」里配置 AI（api_base / api_key / model）".into(),
            ));
            cx.notify();
            return;
        };
        let count: usize = self
            .question_count
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(10);
        if count == 0 || count > 100 {
            self.message = Some(Message::Error("出题数量需在 1-100 之间".into()));
            cx.notify();
            return;
        }
        let content = note.content.clone();
        let pool = AppState::global(cx).pool.clone();
        self.generating = true;
        self.message = None;
        cx.notify();

        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let questions =
                            crate::ai::generate::generate_questions(&ai, &content, count).await?;
                        for q in &questions {
                            let card = Card::new(
                                note_id,
                                q.question.clone(),
                                q.standard_answer.clone(),
                                q.source_excerpt.clone(),
                            );
                            db::cards::insert(&pool, &card).await?;
                        }
                        Ok(questions.len())
                    })
                    .await;
                    match result {
                        Ok(n) => {
                            this.update(&mut cx, |this, cx| {
                                this.generating = false;
                                this.message = Some(Message::Success(format!("已生成 {n} 道题")));
                                this.refresh_cards(cx);
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.generating = false;
                                this.message = Some(Message::Error(format!("出题失败: {e}")));
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
            },
        )
        .detach();
    }
}

impl Render for NotesView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Root 维护弹层状态，但不会主动重绘这个子实体，因此弹层在触发它的视图中挂载。
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        let colors = cx.theme().colors;
        let compact = window.viewport_size().width.as_f32() < 1040.;

        let header = match self.page {
            NotesPage::Library => page_header(
                RuizIcon::NotebookText,
                "资料库",
                "集中管理学习材料，打开一篇笔记后再生成和浏览学习卡片。",
                Some(
                    Button::new("btn-import")
                        .icon(IconName::Plus)
                        .label(if self.saving {
                            "保存中…"
                        } else {
                            "导入材料"
                        })
                        .primary()
                        .loading(self.saving)
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_import_sheet(window, cx);
                        }))
                        .into_any_element(),
                ),
                cx,
            ),
            NotesPage::Detail(note_id) => {
                let title = self
                    .notes
                    .iter()
                    .find(|note| note.id == note_id)
                    .map(|note| note.title.clone())
                    .unwrap_or_else(|| "笔记详情".into());
                page_header(
                    RuizIcon::NotebookText,
                    title,
                    "查看原始材料，生成学习卡片，并管理这篇笔记。",
                    Some(
                        Button::new("back-to-library")
                            .icon(IconName::ArrowLeft)
                            .label("返回资料库")
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| this.show_library(cx)))
                            .into_any_element(),
                    ),
                    cx,
                )
            }
        };

        // 提示消息
        let alerts = self.message.clone().map(|message| {
            let alert: gpui::AnyElement = match message {
                Message::Success(msg) => Alert::success("note-alert", msg)
                    .banner()
                    .on_close(cx.listener(|this, _, _, cx| {
                        this.message = None;
                        cx.notify();
                    }))
                    .into_any_element(),
                Message::Error(msg) => Alert::error("note-alert", msg)
                    .banner()
                    .on_close(cx.listener(|this, _, _, cx| {
                        this.message = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            };
            div().px_6().pt_4().child(alert)
        });

        // 一级资料库：全宽卡片列表，不再常驻挤占详情空间。
        let notes_list = div()
            .id("notes-list-wrap")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                v_flex()
                    .id("notes-list")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.notes_scroll)
                    .child(
                        h_flex()
                            .w_full()
                            .max_w(px(1040.))
                            .mx_auto()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .px_6()
                            .pt_6()
                            .pb_4()
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(div().font_semibold().child("全部材料"))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(colors.muted_foreground)
                                            .child("点击任意材料进入二级详情页"),
                                    ),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded_full()
                                    .bg(colors.muted)
                                    .text_sm()
                                    .text_color(colors.muted_foreground)
                                    .child(format!("{} 篇", self.notes.len())),
                            ),
                    )
                    .when(self.notes_loading, |this| {
                        this.child(
                            v_flex()
                                .w_full()
                                .max_w(px(1040.))
                                .mx_auto()
                                .gap_3()
                                .px_6()
                                .child(Skeleton::new().w_full())
                                .child(Skeleton::new().secondary().w_4_5())
                                .child(Skeleton::new().secondary().w_full()),
                        )
                    })
                    .when(!self.notes_loading && self.notes.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .px_5()
                                .text_center()
                                .text_sm()
                                .text_color(colors.muted_foreground)
                                .child(Icon::new(IconName::Inbox).size_6())
                                .child("还没有笔记"),
                        )
                    })
                    .children(
                        (!self.notes_loading)
                            .then_some(())
                            .into_iter()
                            .flat_map(|_| {
                                self.notes.iter().map(|note| {
                                    let id = note.id;
                                    let title = note.title.clone();
                                    let created_at = note.created_at.format("%Y-%m-%d").to_string();
                                    let excerpt =
                                        preview(&note.content, if compact { 80 } else { 150 });
                                    div()
                                        .id(SharedString::from(format!("library-note-{id}")))
                                        .w_full()
                                        .max_w(px(992.))
                                        .mx_auto()
                                        .mb_3()
                                        .p_4()
                                        .rounded_xl()
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(colors.background)
                                        .cursor_pointer()
                                        .hover(move |style| {
                                            style
                                                .bg(colors.accent.opacity(0.45))
                                                .border_color(colors.primary.opacity(0.45))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_note(id, cx);
                                        }))
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_4()
                                                .child(
                                                    h_flex()
                                                        .size_10()
                                                        .flex_shrink_0()
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded_lg()
                                                        .bg(colors.primary.opacity(0.1))
                                                        .text_color(colors.primary)
                                                        .child(
                                                            Icon::new(RuizIcon::NotebookText)
                                                                .size_5(),
                                                        ),
                                                )
                                                .child(
                                                    v_flex()
                                                        .min_w_0()
                                                        .flex_1()
                                                        .gap_0p5()
                                                        .child(
                                                            div()
                                                                .overflow_hidden()
                                                                .text_ellipsis()
                                                                .font_semibold()
                                                                .child(SharedString::from(title)),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .text_color(colors.muted_foreground)
                                                                .child(excerpt),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(colors.muted_foreground)
                                                                .child(created_at),
                                                        ),
                                                )
                                                .child(
                                                    Icon::new(IconName::ArrowRight)
                                                        .size_4()
                                                        .text_color(colors.muted_foreground),
                                                ),
                                        )
                                })
                            }),
                    ),
            )
            .vertical_scrollbar(&self.notes_scroll);

        // 二级页面：只展示当前笔记详情。
        let detail = if let Some(id) = self.page.note_id() {
            let note = self.notes.iter().find(|n| n.id == id);
            let cards = &self.cards;
            let breadcrumb_view = cx.entity();
            let breadcrumb = Breadcrumb::new()
                .child(BreadcrumbItem::new("资料库").on_click(move |_, _, cx| {
                    breadcrumb_view.update(cx, |this, cx| this.show_library(cx));
                }))
                .child(BreadcrumbItem::new(
                    note.map(|note| note.title.clone())
                        .unwrap_or_else(|| "笔记详情".into()),
                ));
            let generate_bar = GroupBox::new()
                .outline()
                .title(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(RuizIcon::Sparkles).size_4())
                        .child("AI 出题"),
                )
                .child(
                    h_flex()
                        .items_end()
                        .justify_between()
                        .gap_4()
                        .flex_wrap()
                        .when(compact, |this| this.flex_col().items_start())
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("每次生成"))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(72.))
                                                .child(Input::new(&self.question_count)),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(colors.muted_foreground)
                                                .child("张卡片"),
                                        ),
                                ),
                        )
                        .child(
                            Button::new("btn-generate")
                                .icon(RuizIcon::Sparkles)
                                .label(if self.generating {
                                    "生成中…"
                                } else {
                                    "开始生成"
                                })
                                .primary()
                                .loading(self.generating)
                                .disabled(self.generating)
                                .when(compact, |this| this.w_full())
                                .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                        ),
                );

            let card_list = if self.cards_loading {
                v_flex()
                    .w_full()
                    .gap_3()
                    .child(Skeleton::new().w_full())
                    .child(Skeleton::new().secondary().w_4_5())
                    .child(Skeleton::new().secondary().w_full())
                    .into_any_element()
            } else if cards.is_empty() {
                empty_state(
                    IconName::Inbox,
                    "还没有学习卡片",
                    "用 AI 从这篇笔记生成一组问题，之后就可以在复习页练习。",
                    None,
                    cx,
                )
                .into_any_element()
            } else {
                v_flex()
                    .w_full()
                    .gap_2()
                    .children(cards.iter().enumerate().map(|(index, card)| {
                        div()
                            .w_full()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.background)
                            .hover(|style| style.border_color(colors.primary))
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .size_6()
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .rounded_full()
                                            .bg(colors.muted)
                                            .text_xs()
                                            .text_color(colors.muted_foreground)
                                            .child((index + 1).to_string()),
                                    )
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .flex_1()
                                            .gap_1()
                                            .child(SharedString::from(card.question.clone()))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(colors.muted_foreground)
                                                    .child(format!(
                                                        "复习 {} 次 · 遗忘 {} 次",
                                                        card.reps, card.lapses
                                                    )),
                                            ),
                                    ),
                            )
                    }))
                    .into_any_element()
            };

            v_flex()
                .flex_1()
                .w_full()
                .max_w(px(1040.))
                .mx_auto()
                .when(compact, |this| this.p_4())
                .when(!compact, |this| this.p_6())
                .gap_4()
                .child(breadcrumb)
                .child(
                    h_flex()
                        .items_start()
                        .justify_between()
                        .gap_4()
                        .flex_wrap()
                        .child(
                            v_flex()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_2xl()
                                        .font_semibold()
                                        .text_color(colors.foreground)
                                        .child(SharedString::from(
                                            note.map(|n| n.title.clone()).unwrap_or_default(),
                                        )),
                                )
                                .child(
                                    div().text_sm().text_color(colors.muted_foreground).child(
                                        note.map(|n| {
                                            format!(
                                                "创建于 {} · {} 个字符",
                                                n.created_at.format("%Y-%m-%d"),
                                                n.content.chars().count()
                                            )
                                        })
                                        .unwrap_or_default(),
                                    ),
                                ),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .rounded_full()
                                        .bg(colors.muted)
                                        .text_sm()
                                        .text_color(colors.muted_foreground)
                                        .child(format!("{} 张卡片", cards.len())),
                                )
                                .child(
                                    Button::new("delete-note")
                                        .small()
                                        .icon(IconName::Delete)
                                        .label("删除")
                                        .danger()
                                        .outline()
                                        .loading(self.deleting_note_id == Some(id))
                                        .disabled(self.deleting_note_id.is_some())
                                        .on_click({
                                            let title = note
                                                .map(|note| note.title.clone())
                                                .unwrap_or_default();
                                            cx.listener(move |this, _, window, cx| {
                                                this.confirm_delete_note(
                                                    id,
                                                    title.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })
                                        }),
                                ),
                        ),
                )
                .child(generate_bar)
                .child(
                    GroupBox::new()
                        .fill()
                        .title(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::File).size_4())
                                .child("原始材料"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted_foreground)
                                .child(note.map(|n| preview(&n.content, 320)).unwrap_or_default()),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(RuizIcon::NotebookText).size_4())
                                .child(div().font_semibold().child("学习卡片")),
                        )
                        .child(card_list),
                )
                .into_any_element()
        } else {
            empty_state(
                RuizIcon::NotebookText,
                "选择一篇笔记开始",
                "从左侧打开学习材料，或使用右上角的“导入材料”创建第一篇笔记。",
                None,
                cx,
            )
            .into_any_element()
        };

        // 右侧详情滚动容器（外层 relative 挂滚动条，滚动条不随内容滚动）
        let detail_scrollable = div()
            .id("detail-scroll-wrap")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                div()
                    .id("detail-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.detail_scroll)
                    .child(detail),
            )
            .vertical_scrollbar(&self.detail_scroll);

        let page = match self.page {
            NotesPage::Library => notes_list.into_any_element(),
            NotesPage::Detail(_) => detail_scrollable.into_any_element(),
        };

        div()
            .relative()
            .size_full()
            .child(
                v_flex()
                    .size_full()
                    .child(header)
                    .children(alerts)
                    .child(page),
            )
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}

fn preview(content: &str, limit: usize) -> String {
    let mut preview = content.chars().take(limit).collect::<String>();
    if content.chars().count() > limit {
        preview.push('…');
    }
    preview
}
