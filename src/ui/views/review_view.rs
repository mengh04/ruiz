//! 复习视图：展示到期卡片 → 用户敲字作答 → AI 判官 → FSRS 调度。

use chrono::Utc;
use fsrs::NextStates;
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
use gpui_component::skeleton::Skeleton;
use gpui_component::theme::{ActiveTheme as _, ThemeColor};
use gpui_component::{Icon, IconName, StyledExt as _, h_flex, v_flex};

use crate::ai::judge::Judgement;
use crate::assets::RuizIcon;
use crate::db;
use crate::domain::card::Card;
use crate::domain::review::Rating;
use crate::scheduler::Scheduler;
use crate::state::AppState;
use crate::ui::components::{empty_state, page_header};

enum Phase {
    Loading,
    Answering,
    Judging,
    Judged {
        judgement: Judgement,
        next: NextStates,
    },
    Finished,
}

pub struct ReviewView {
    queue: Vec<Card>,
    current: Option<Card>,
    phase: Phase,
    answer: Entity<InputState>,
    error: Option<String>,
    show_source: bool,
    submitted_answer: String,
    /// 窗口句柄（清空输入框等窗口操作需要）
    window: AnyWindowHandle,
    /// 内容区滚动
    scroll: ScrollHandle,
}

impl ReviewView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let window_handle = window.window_handle();
        let answer = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("在这里敲出你的答案…（尽量回忆，先别看原文）")
                .multi_line(true)
                .rows(6)
        });
        let mut view = Self {
            queue: Vec::new(),
            current: None,
            phase: Phase::Loading,
            answer,
            error: None,
            show_source: false,
            submitted_answer: String::new(),
            window: window_handle,
            scroll: ScrollHandle::new(),
        };
        view.load(cx);
        view
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let pool = AppState::global(cx).pool.clone();
        self.phase = Phase::Loading;
        self.error = None;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::cards::due(&pool, Utc::now()).await
                    })
                    .await;
                    match result {
                        Ok(queue) => {
                            this.update(&mut cx, |this, cx| {
                                this.queue = queue;
                                this.current = this.queue.first().cloned();
                                this.phase = if this.current.is_some() {
                                    Phase::Answering
                                } else {
                                    Phase::Finished
                                };
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.error = Some(format!("加载复习队列失败: {e}"));
                                this.phase = Phase::Finished;
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

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.current.clone() else {
            return;
        };
        let Some(ai) = AppState::global(cx).ai.clone() else {
            self.error = Some("请先在「设置」里配置 AI（api_base / api_key / model）".into());
            cx.notify();
            return;
        };
        let user_answer = self.answer.read(cx).value().to_string();
        if user_answer.trim().is_empty() {
            self.error = Some("答案不能为空".into());
            cx.notify();
            return;
        }
        self.phase = Phase::Judging;
        self.error = None;
        cx.notify();
        let (question, standard_answer) = (card.question.clone(), card.standard_answer.clone());
        self.submitted_answer = user_answer.clone();
        let submitted = user_answer;
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        crate::ai::judge::judge(&ai, &question, &standard_answer, &submitted).await
                    })
                    .await;
                    match result {
                        Ok(judgement) => {
                            this.update(&mut cx, |this, cx| {
                                let days = card.days_elapsed(Utc::now());
                                let next = AppState::global(cx)
                                    .scheduler
                                    .next_states(card.memory, days);
                                match next {
                                    Ok(next) => {
                                        this.phase = Phase::Judged { judgement, next };
                                    }
                                    Err(e) => {
                                        this.phase = Phase::Answering;
                                        this.error = Some(format!("FSRS 计算失败: {e}"));
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.phase = Phase::Answering;
                                this.error = Some(format!("判官调用失败: {e}"));
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

    fn rate(&mut self, rating: Rating, cx: &mut Context<Self>) {
        let Some(card) = self.current.clone() else {
            return;
        };
        let (judgement, next) = match &self.phase {
            Phase::Judged { judgement, next } => (judgement.clone(), next.clone()),
            _ => return,
        };
        let state = Scheduler::state_for(rating, &next);
        let due = Scheduler::due_date(state);
        let reps = card.reps + 1;
        let lapses = if rating == Rating::Again {
            card.lapses + 1
        } else {
            card.lapses
        };
        let memory = state.memory;
        let pool = AppState::global(cx).pool.clone();
        let user_answer = self.submitted_answer.clone();
        let feedback = judgement.feedback.clone();
        let answer_input = self.answer.clone();
        let window_handle = self.window;
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::cards::update_schedule(&pool, card.id, memory, due, reps, lapses)
                            .await?;
                        db::reviews::insert(&pool, card.id, &user_answer, &feedback, rating)
                            .await?;
                        Ok(())
                    })
                    .await;
                    match result {
                        Ok(()) => {
                            // 清空答案输入框
                            cx.update_window(window_handle, move |_view, window, cx| {
                                answer_input.update(cx, |s, cx| s.set_value("", window, cx));
                            })
                            .ok();
                            this.update(&mut cx, |this, cx| {
                                this.queue.retain(|c| c.id != card.id);
                                this.current = this.queue.first().cloned();
                                this.show_source = false;
                                this.submitted_answer.clear();
                                this.phase = if this.current.is_some() {
                                    Phase::Answering
                                } else {
                                    Phase::Finished
                                };
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            this.update(&mut cx, |this, cx| {
                                this.error = Some(format!("保存复习记录失败: {e}"));
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

impl Render for ReviewView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;

        let error = self.error.clone().map(|e| {
            Alert::error("review-alert", e)
                .banner()
                .on_close(cx.listener(|this, _, _, cx| {
                    this.error = None;
                    cx.notify();
                }))
                .into_any_element()
        });

        let header = page_header(
            RuizIcon::BrainCircuit,
            "复习",
            "专注回忆，再让 AI 给出反馈，最后交给 FSRS 安排下一次复习。",
            Some(
                Button::new("btn-reload-header")
                    .icon(IconName::Redo2)
                    .label("刷新队列")
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                    .into_any_element(),
            ),
            cx,
        );

        let body: gpui::AnyElement = match &self.phase {
            Phase::Loading => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_6()
                .child(
                    GroupBox::new()
                        .outline()
                        .w_full()
                        .max_w(px(680.))
                        .title(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(RuizIcon::BrainCircuit).size_4())
                                .child("正在准备复习队列"),
                        )
                        .child(Skeleton::new().w_2_3())
                        .child(Skeleton::new().secondary().w_full())
                        .child(Skeleton::new().secondary().w_4_5()),
                ),
            Phase::Finished => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .children(error)
                .child(empty_state(
                    IconName::CircleCheck,
                    "今天的复习已经完成",
                    "到期卡片都处理完了，可以稍后再回来，或刷新检查新的复习任务。",
                    Some(
                        Button::new("btn-reload")
                            .icon(IconName::Redo2)
                            .label("刷新队列")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                            .into_any_element(),
                    ),
                    cx,
                )),
            Phase::Judging => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .p_6()
                .child(
                    GroupBox::new()
                        .fill()
                        .w_full()
                        .max_w(px(680.))
                        .title(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(RuizIcon::Sparkles).size_4())
                                .child("AI 正在评估答案"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.muted_foreground)
                                .child("正在对比问题、标准答案和你的作答，请稍候。"),
                        )
                        .child(Skeleton::new().w_full())
                        .child(Skeleton::new().secondary().w_3_5()),
                ),
            Phase::Answering | Phase::Judged { .. } => {
                let card = self.current.clone().expect("作答阶段必有当前卡片");

                let is_judged = matches!(self.phase, Phase::Judged { .. });
                let question = GroupBox::new()
                    .outline()
                    .title(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::new(RuizIcon::BrainCircuit).size_4())
                                    .child("问题"),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(colors.muted)
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(format!("剩余 {} 张", self.queue.len())),
                            ),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_medium()
                            .child(SharedString::from(card.question.clone())),
                    );

                // 原文（可折叠）
                let source = if self.show_source {
                    let excerpt = card
                        .source_excerpt
                        .clone()
                        .unwrap_or_else(|| "（本题没有原文引用）".into());
                    let standard = card.standard_answer.clone();
                    GroupBox::new()
                        .fill()
                        .title(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::BookOpen).size_4())
                                .child("参考材料"),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("原文片段"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted_foreground)
                                        .child(SharedString::from(excerpt)),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("标准答案"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted_foreground)
                                        .child(SharedString::from(standard)),
                                ),
                        )
                        .into_any_element()
                } else {
                    div().into_any_element()
                };

                // 判官结果区
                let judge_area = if let Phase::Judged { judgement, next } = &self.phase {
                    Some(
                        GroupBox::new()
                            .outline()
                            .border_color(colors.primary)
                            .title(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(Icon::new(RuizIcon::Sparkles).size_4())
                                            .child("AI 反馈"),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(colors.primary.opacity(0.12))
                                            .text_sm()
                                            .font_semibold()
                                            .text_color(colors.primary)
                                            .child(format!("{}/100", judgement.score)),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_medium()
                                            .child(SharedString::from(judgement.rating.clone())),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(colors.muted_foreground)
                                            .child(SharedString::from(judgement.feedback.clone())),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(rating_button(
                                        cx,
                                        Rating::Again,
                                        format!("重来 · {}", interval_label(next.again.interval)),
                                    ))
                                    .child(rating_button(
                                        cx,
                                        Rating::Hard,
                                        format!("困难 · {}", interval_label(next.hard.interval)),
                                    ))
                                    .child(rating_button(
                                        cx,
                                        Rating::Good,
                                        format!("良好 · {}", interval_label(next.good.interval)),
                                    ))
                                    .child(rating_button(
                                        cx,
                                        Rating::Easy,
                                        format!("简单 · {}", interval_label(next.easy.interval)),
                                    )),
                            ),
                    )
                } else {
                    None
                };

                // 作答区
                let answer_area = GroupBox::new()
                    .fill()
                    .title(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::SquareTerminal).size_4())
                            .child("你的回答"),
                    )
                    .child(Input::new(&self.answer))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                Button::new("btn-show-source")
                                    .icon(if self.show_source {
                                        IconName::EyeOff
                                    } else {
                                        IconName::Eye
                                    })
                                    .label(if self.show_source {
                                        "隐藏参考材料"
                                    } else {
                                        "显示参考材料"
                                    })
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_source = !this.show_source;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("btn-submit")
                                    .icon(IconName::Check)
                                    .label(if is_judged {
                                        "已完成判分"
                                    } else {
                                        "提交判分"
                                    })
                                    .primary()
                                    .disabled(is_judged)
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            ),
                    );

                v_flex()
                    .w_full()
                    .max_w(px(880.))
                    .mx_auto()
                    .p_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(step_badge("1", "主动回忆", true, colors))
                            .child(div().h(px(1.)).flex_1().bg(colors.border))
                            .child(step_badge("2", "AI 判分", is_judged, colors))
                            .child(div().h(px(1.)).flex_1().bg(colors.border))
                            .child(step_badge("3", "选择排期", is_judged, colors)),
                    )
                    .child(question)
                    .child(source)
                    .children(error)
                    .child(answer_area)
                    .children(judge_area)
            }
        }
        .into_any_element();

        v_flex().size_full().child(header).child(
            div()
                .id("review-scroll-wrap")
                .relative()
                .flex_1()
                .h_full()
                .child(
                    div()
                        .id("review-scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(body),
                )
                .vertical_scrollbar(&self.scroll),
        )
    }
}

fn step_badge(
    number: &'static str,
    label: &'static str,
    active: bool,
    colors: ThemeColor,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_1()
        .when(active, |this| this.text_color(colors.primary))
        .child(
            h_flex()
                .size_5()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(if active { colors.primary } else { colors.muted })
                .text_color(if active {
                    colors.primary_foreground
                } else {
                    colors.muted_foreground
                })
                .child(number),
        )
        .child(label)
}

fn interval_label(days: f32) -> String {
    if days < 1.0 {
        "< 1 天".into()
    } else {
        format!("{days:.0} 天")
    }
}

fn rating_button(
    cx: &mut Context<ReviewView>,
    rating: Rating,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    let id = match rating {
        Rating::Again => "rate-again",
        Rating::Hard => "rate-hard",
        Rating::Good => "rate-good",
        Rating::Easy => "rate-easy",
    };
    let icon = match rating {
        Rating::Again => IconName::Undo2,
        Rating::Hard => IconName::TriangleAlert,
        Rating::Good => IconName::ThumbsUp,
        Rating::Easy => IconName::Star,
    };
    let mut button = Button::new(id).icon(icon).label(label);
    button = match rating {
        Rating::Again => button.danger(),
        Rating::Hard => button.warning(),
        Rating::Good => button.success(),
        Rating::Easy => button.primary(),
    };
    button.on_click(cx.listener(move |this, _, _, cx| this.rate(rating, cx)))
}
