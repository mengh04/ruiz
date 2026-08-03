//! 笔记视图：导入学习材料、AI 出题、浏览卡片。

use gpui::{
    AnyWindowHandle, Context, Entity, IntoElement, Render, ScrollHandle, SharedString, Window, div,
    prelude::*, px,
};
use gpui_component::Disableable as _;
use gpui_component::alert::Alert;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::theme::ActiveTheme as _;
use gpui_component::{Icon, IconName, StyledExt as _, h_flex, v_flex};

use crate::assets::RuizIcon;
use crate::db;
use crate::domain::card::Card;
use crate::domain::note::Note;
use crate::state::AppState;
use crate::ui::components::{empty_state, page_header};

pub struct NotesView {
    notes: Vec<Note>,
    selected_note_id: Option<i64>,
    cards: Vec<Card>,
    /// 是否显示导入表单
    importing: bool,
    title: Entity<InputState>,
    content: Entity<InputState>,
    question_count: Entity<InputState>,
    generating: bool,
    message: Option<Message>,
    /// 窗口句柄（清空输入框等窗口操作需要）
    window: AnyWindowHandle,
    /// 笔记列表滚动
    notes_scroll: ScrollHandle,
    /// 详情区滚动
    detail_scroll: ScrollHandle,
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
            selected_note_id: None,
            cards: Vec::new(),
            importing: false,
            title,
            content,
            question_count,
            generating: false,
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
                                // 保持选中项；若被删除则清空
                                if let Some(id) = this.selected_note_id
                                    && !this.notes.iter().any(|n| n.id == id)
                                {
                                    this.selected_note_id = None;
                                    this.cards.clear();
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
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
        let Some(note_id) = self.selected_note_id else {
            return;
        };
        let pool = AppState::global(cx).pool.clone();
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
                                this.cards = cards;
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.message = Some(Message::Error(format!("加载卡片失败: {e}")));
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

    fn save_note(&mut self, cx: &mut Context<Self>) {
        let pool = AppState::global(cx).pool.clone();
        let title = self.title.read(cx).value().trim().to_string();
        let content = self.content.read(cx).value().to_string();
        if title.is_empty() || content.trim().is_empty() {
            self.message = Some(Message::Error("标题和内容都不能为空".into()));
            cx.notify();
            return;
        }
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
                            })
                            .ok();
                            this.update(&mut cx, |this, cx| {
                                this.importing = false;
                                this.selected_note_id = Some(id);
                                this.message = Some(Message::Success("笔记已保存".into()));
                                this.refresh_notes(cx);
                                this.refresh_cards(cx);
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
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
    }

    fn select_note(&mut self, id: i64, cx: &mut Context<Self>) {
        self.selected_note_id = Some(id);
        self.refresh_cards(cx);
        cx.notify();
    }

    /// AI 出题：把选中笔记拆成若干道题并入库
    fn generate(&mut self, cx: &mut Context<Self>) {
        let Some(note_id) = self.selected_note_id else {
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
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let sidebar = cx.theme().sidebar;
        let sidebar_accent = cx.theme().sidebar_accent;

        let import_icon = if self.importing {
            IconName::ChevronUp
        } else {
            IconName::Plus
        };
        let header = page_header(
            RuizIcon::NotebookText,
            "笔记",
            "整理学习材料，生成卡片，并把知识带入间隔复习流程。",
            Some(
                Button::new("btn-import")
                    .icon(import_icon)
                    .label(if self.importing {
                        "收起导入"
                    } else {
                        "导入材料"
                    })
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.importing = !this.importing;
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            cx,
        );

        // 导入表单
        let import_form = if self.importing {
            Some(
                div().px_6().pt_4().child(
                    GroupBox::new()
                        .fill()
                        .title(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::Plus).size_4())
                                .child("导入学习材料"),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("标题"))
                                .child(Input::new(&self.title)),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("正文内容"))
                                .child(Input::new(&self.content)),
                        )
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("btn-cancel-import")
                                        .label("取消")
                                        .outline()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.importing = false;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("btn-save-note")
                                        .icon(IconName::Check)
                                        .label("保存笔记")
                                        .primary()
                                        .on_click(cx.listener(|this, _, _, cx| this.save_note(cx))),
                                ),
                        ),
                ),
            )
        } else {
            None
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

        // 笔记列表（外层 relative 容器挂滚动条，滚动条不随内容滚动）
        let notes_list = div()
            .id("notes-list-wrap")
            .relative()
            .w(px(288.))
            .h_full()
            .flex_shrink_0()
            .bg(sidebar)
            .border_r_1()
            .border_color(colors.border)
            .child(
                v_flex()
                    .id("notes-list")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.notes_scroll)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_3()
                            .child(div().text_sm().font_semibold().child("资料库"))
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(colors.muted)
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(self.notes.len().to_string()),
                            ),
                    )
                    .when(self.notes.is_empty(), |this| {
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
                    .children(self.notes.iter().map(|note| {
                        let active = self.selected_note_id == Some(note.id);
                        let id = note.id;
                        let title = note.title.clone();
                        let created_at = note.created_at.format("%Y-%m-%d").to_string();
                        div()
                            .id(SharedString::from(format!("note-{id}")))
                            .mx_2()
                            .mb_1()
                            .p_2()
                            .rounded_md()
                            .cursor_pointer()
                            .when(active, |this| {
                                this.bg(sidebar_accent)
                                    .text_color(cx.theme().sidebar_accent_foreground)
                            })
                            .hover(move |style| {
                                style.bg(sidebar_accent.opacity(if active { 1.0 } else { 0.72 }))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_note(id, cx);
                            }))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        h_flex()
                                            .size_8()
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .rounded_md()
                                            .bg(colors.muted)
                                            .text_color(colors.muted_foreground)
                                            .child(Icon::new(RuizIcon::NotebookText).size_4()),
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
                                                    .text_sm()
                                                    .font_medium()
                                                    .child(SharedString::from(title)),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(colors.muted_foreground)
                                                    .child(created_at),
                                            ),
                                    ),
                            )
                    })),
            )
            .vertical_scrollbar(&self.notes_scroll);

        // 右侧：选中笔记详情 / 提示
        let detail = if let Some(id) = self.selected_note_id {
            let note = self.notes.iter().find(|n| n.id == id);
            let cards = &self.cards;
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
                                .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                        ),
                );

            let card_list = if cards.is_empty() {
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
                .max_w(px(920.))
                .mx_auto()
                .p_6()
                .gap_4()
                .child(
                    h_flex()
                        .items_start()
                        .justify_between()
                        .gap_4()
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
                            div()
                                .px_2()
                                .py_1()
                                .rounded_full()
                                .bg(colors.muted)
                                .text_sm()
                                .text_color(colors.muted_foreground)
                                .child(format!("{} 张卡片", cards.len())),
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
            .h_full()
            .child(
                div()
                    .id("detail-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.detail_scroll)
                    .child(detail),
            )
            .vertical_scrollbar(&self.detail_scroll);

        v_flex()
            .size_full()
            .child(header)
            .children(import_form)
            .children(alerts)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(notes_list)
                    .child(detail_scrollable),
            )
    }
}

fn preview(content: &str, limit: usize) -> String {
    let mut preview = content.chars().take(limit).collect::<String>();
    if content.chars().count() > limit {
        preview.push('…');
    }
    preview
}
