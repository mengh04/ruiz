//! 动态复习视图：知识单元调度 → 自适应题型 → AI 动态题面 → 判分 → FSRS。

use chrono::Utc;
use fsrs::NextStates;
use gpui::{
    AnyWindowHandle, Context, Entity, IntoElement, Render, ScrollHandle, SharedString, Window, div,
    prelude::*, px,
};
use gpui_component::Disableable as _;
use gpui_component::alert::Alert;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_component::skeleton::Skeleton;
use gpui_component::spinner::Spinner;
use gpui_component::theme::{ActiveTheme as _, ThemeColor};
use gpui_component::{
    Icon, IconName, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};

use crate::ai::client::ChatClient;
use crate::ai::judge::Judgement;
use crate::assets::RuizIcon;
use crate::db;
use crate::domain::dynamic_review::{QuestionFormat, ReviewItem, ReviewPrompt};
use crate::domain::note::Note;
use crate::domain::review::Rating;
use crate::scheduler::Scheduler;
use crate::settings::AppSettings;
use crate::state::AppState;
use crate::ui::components::{empty_state, page_header};

enum Phase {
    Loading,
    Ready,
    Answering,
    Judging,
    Scheduling,
    Judged {
        judgement: Judgement,
        next: NextStates,
    },
    Finished,
}

#[derive(Clone)]
struct PreparedPrompt {
    prompt: ReviewPrompt,
    notice: Option<String>,
    adaptive_answer_formats: bool,
}

#[derive(Clone)]
struct ReviewGroupChoice {
    id: i64,
    label: SharedString,
    name: SharedString,
}

impl SearchableListItem for ReviewGroupChoice {
    type Value = i64;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(&query.to_lowercase())
    }
}

pub struct ReviewView {
    queue: Vec<ReviewItem>,
    notes: Vec<Note>,
    selected_group_ids: Vec<i64>,
    group_filter: Entity<ComboboxState<SearchableVec<ReviewGroupChoice>>>,
    selected_note_id: Option<i64>,
    current: Option<ReviewItem>,
    prompt: Option<ReviewPrompt>,
    selected_option: Option<String>,
    phase: Phase,
    answer: Entity<InputState>,
    error: Option<String>,
    generation_notice: Option<String>,
    prefetched: Option<PreparedPrompt>,
    preparing_unit_id: Option<i64>,
    prefetching_unit_id: Option<i64>,
    load_revision: u64,
    show_source: bool,
    submitted_answer: String,
    window: AnyWindowHandle,
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
        let group_filter = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::new(Vec::<ReviewGroupChoice>::new()),
                Vec::new(),
                window,
                cx,
            )
            .multiple(true)
            .searchable(true)
        });
        let mut view = Self {
            queue: Vec::new(),
            notes: Vec::new(),
            selected_group_ids: Vec::new(),
            group_filter: group_filter.clone(),
            selected_note_id: None,
            current: None,
            prompt: None,
            selected_option: None,
            phase: Phase::Loading,
            answer,
            error: None,
            generation_notice: None,
            prefetched: None,
            preparing_unit_id: None,
            prefetching_unit_id: None,
            load_revision: 0,
            show_source: false,
            submitted_answer: String::new(),
            window: window_handle,
            scroll: ScrollHandle::new(),
        };
        cx.subscribe(&group_filter, |this, _, event, cx| {
            if let ComboboxEvent::Change(group_ids) = event {
                this.selected_group_ids = group_ids.clone();
                this.selected_note_id = None;
                this.load(cx);
            }
        })
        .detach();
        view.load(cx);
        view
    }

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        let pool = AppState::global(cx).pool.clone();
        let group_ids = self.selected_group_ids.clone();
        let note_id = self.selected_note_id;
        self.load_revision = self.load_revision.wrapping_add(1);
        let revision = self.load_revision;
        self.phase = Phase::Loading;
        self.queue.clear();
        self.current = None;
        self.prompt = None;
        self.prefetched = None;
        self.preparing_unit_id = None;
        self.prefetching_unit_id = None;
        self.error = None;
        self.generation_notice = None;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let groups = db::groups::summaries(&pool, Utc::now()).await?;
                        let notes = db::notes::list(&pool).await?;
                        let queue = db::dynamic_reviews::due_in_scope(
                            &pool,
                            Utc::now(),
                            &group_ids,
                            note_id,
                        )
                        .await?;
                        anyhow::Ok((groups, notes, queue))
                    })
                    .await;
                    match result {
                        Ok((groups, notes, queue)) => {
                            let choices = groups
                                .iter()
                                .map(|summary| ReviewGroupChoice {
                                    id: summary.group.id,
                                    label: format!(
                                        "{} · {} 项 · {} 到期",
                                        summary.group.name, summary.card_count, summary.due_count
                                    )
                                    .into(),
                                    name: summary.group.name.clone().into(),
                                })
                                .collect::<Vec<_>>();
                            let filter_state = this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision {
                                    return None;
                                }
                                this.selected_group_ids.retain(|id| {
                                    groups.iter().any(|summary| summary.group.id == *id)
                                });
                                if this.selected_note_id.is_some_and(|id| {
                                    !notes.iter().any(|note| {
                                        note.id == id
                                            && (this.selected_group_ids.is_empty()
                                                || this.selected_group_ids.contains(&note.group_id))
                                    })
                                }) {
                                    this.selected_note_id = None;
                                }
                                this.notes = notes;
                                this.queue = queue;
                                this.current = this.queue.first().cloned();
                                if this.current.is_some() {
                                    // 先停在 Ready：等用户点击「开始复习」才调用 AI 生成题面。
                                    this.phase = Phase::Ready;
                                    cx.notify();
                                } else {
                                    this.phase = Phase::Finished;
                                    cx.notify();
                                }
                                Some((
                                    this.group_filter.clone(),
                                    this.selected_group_ids.clone(),
                                    this.window,
                                ))
                            });
                            if let Ok(Some((group_filter, selected, window_handle))) = filter_state
                            {
                                cx.update_window(window_handle, move |_, window, cx| {
                                    group_filter.update(cx, |state, cx| {
                                        state.set_items(SearchableVec::new(choices), window, cx);
                                        state.set_selected_values(&selected, window, cx);
                                    });
                                })
                                .ok();
                            }
                        }
                        Err(error) => {
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision {
                                    return;
                                }
                                this.error = Some(format!("加载复习队列失败: {error}"));
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

    fn prepare_current(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current.clone() else {
            self.phase = Phase::Finished;
            cx.notify();
            return;
        };
        let adaptive_answer_formats = AppSettings::global(cx)
            .settings
            .review
            .adaptive_answer_formats;
        if self.prefetched.as_ref().is_some_and(|prepared| {
            prepared.prompt.unit_id == item.unit_id
                && prepared.adaptive_answer_formats == adaptive_answer_formats
        }) {
            let prepared = self.prefetched.take().expect("已检查预生成题目");
            self.prompt = Some(prepared.prompt);
            self.generation_notice = prepared.notice;
            self.phase = Phase::Answering;
            self.prefetch_next(cx);
            cx.notify();
            return;
        }
        if self
            .prefetched
            .as_ref()
            .is_some_and(|prepared| prepared.prompt.unit_id == item.unit_id)
        {
            self.prefetched = None;
        }
        if self.prefetching_unit_id == Some(item.unit_id) {
            self.phase = Phase::Loading;
            cx.notify();
            return;
        }
        let pool = AppState::global(cx).pool.clone();
        let ai = AppState::global(cx).ai.clone();
        let revision = self.load_revision;
        let unit_id = item.unit_id;
        self.preparing_unit_id = Some(unit_id);
        self.phase = Phase::Loading;
        self.prompt = None;
        self.selected_option = None;
        self.show_source = false;
        self.generation_notice = None;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        prepare_prompt(pool, ai, item, adaptive_answer_formats).await
                    })
                    .await;
                    match result {
                        Ok(prepared) => {
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision
                                    || this.preparing_unit_id != Some(prepared.prompt.unit_id)
                                {
                                    return;
                                }
                                this.preparing_unit_id = None;
                                let current_setting = AppSettings::global(cx)
                                    .settings
                                    .review
                                    .adaptive_answer_formats;
                                if prepared.adaptive_answer_formats != current_setting {
                                    this.prepare_current(cx);
                                    return;
                                }
                                if this.current.as_ref().map(|item| item.unit_id)
                                    == Some(prepared.prompt.unit_id)
                                {
                                    this.prompt = Some(prepared.prompt);
                                    this.generation_notice = prepared.notice;
                                    this.phase = Phase::Answering;
                                    this.prefetch_next(cx);
                                    cx.notify();
                                }
                            })
                            .ok();
                        }
                        Err(error) => {
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision
                                    || this.preparing_unit_id != Some(unit_id)
                                {
                                    return;
                                }
                                this.preparing_unit_id = None;
                                this.error = Some(format!("准备动态题目失败: {error}"));
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

    fn prefetch_next(&mut self, cx: &mut Context<Self>) {
        if self.prefetched.is_some() || self.prefetching_unit_id.is_some() {
            return;
        }
        let Some(item) = self.queue.get(1).cloned() else {
            return;
        };
        let pool = AppState::global(cx).pool.clone();
        let ai = AppState::global(cx).ai.clone();
        let adaptive_answer_formats = AppSettings::global(cx)
            .settings
            .review
            .adaptive_answer_formats;
        let revision = self.load_revision;
        let unit_id = item.unit_id;
        self.prefetching_unit_id = Some(unit_id);
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        prepare_prompt(pool, ai, item, adaptive_answer_formats).await
                    })
                    .await;
                    match result {
                        Ok(prepared) => {
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision
                                    || this.prefetching_unit_id != Some(unit_id)
                                {
                                    return;
                                }
                                this.prefetching_unit_id = None;
                                let current_setting = AppSettings::global(cx)
                                    .settings
                                    .review
                                    .adaptive_answer_formats;
                                if prepared.adaptive_answer_formats != current_setting {
                                    if this.current.as_ref().map(|item| item.unit_id)
                                        == Some(unit_id)
                                        && this.prompt.is_none()
                                    {
                                        this.prepare_current(cx);
                                    } else if this.queue.get(1).map(|item| item.unit_id)
                                        == Some(unit_id)
                                    {
                                        this.prefetch_next(cx);
                                    }
                                    return;
                                }
                                if this.current.as_ref().map(|item| item.unit_id) == Some(unit_id)
                                    && this.prompt.is_none()
                                {
                                    this.prompt = Some(prepared.prompt);
                                    this.generation_notice = prepared.notice;
                                    this.phase = Phase::Answering;
                                    this.prefetch_next(cx);
                                    cx.notify();
                                } else if this.queue.get(1).map(|item| item.unit_id)
                                    == Some(unit_id)
                                {
                                    this.prefetched = Some(prepared);
                                }
                            })
                            .ok();
                        }
                        Err(error) => {
                            crate::diagnostics::warn(
                                "review.question.prefetch_failed",
                                "Next dynamic question could not be prefetched",
                                serde_json::json!({
                                    "unit_id": unit_id,
                                    "error": format!("{error:#}"),
                                }),
                            );
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision
                                    || this.prefetching_unit_id != Some(unit_id)
                                {
                                    return;
                                }
                                this.prefetching_unit_id = None;
                                if this.current.as_ref().map(|item| item.unit_id) == Some(unit_id)
                                    && this.prompt.is_none()
                                {
                                    this.prepare_current(cx);
                                }
                            })
                            .ok();
                        }
                    }
                }
            },
        )
        .detach();
    }

    /// 用户点击「开始复习」后，才为当前知识单元生成动态题面。
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.current.is_some() {
            self.prepare_current(cx);
        }
    }

    fn select_note(&mut self, note_id: Option<i64>, cx: &mut Context<Self>) {
        self.selected_note_id = note_id;
        self.load(cx);
    }

    fn select_option(&mut self, option: String, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Answering) {
            self.selected_option = Some(option);
            self.error = None;
            cx.notify();
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current.clone() else {
            return;
        };
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let Some(ai) = AppState::global(cx).ai.clone() else {
            self.error = Some("请先在「设置」里配置 DeepSeek API 密钥".into());
            cx.notify();
            return;
        };
        let user_answer = if prompt.format == QuestionFormat::Choice {
            self.selected_option.clone().unwrap_or_default()
        } else {
            self.answer.read(cx).value().to_string()
        };
        if user_answer.trim().is_empty() {
            self.error = Some(if prompt.format == QuestionFormat::Choice {
                "请先选择一个答案".into()
            } else {
                "答案不能为空".into()
            });
            cx.notify();
            return;
        }
        self.phase = Phase::Judging;
        self.error = None;
        self.submitted_answer = user_answer.clone();
        let revision = self.load_revision;
        let unit_id = item.unit_id;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        crate::ai::judge::judge(
                            &ai,
                            &prompt.question,
                            &prompt.standard_answer,
                            &prompt.required_points,
                            &user_answer,
                        )
                        .await
                    })
                    .await;
                    match result {
                        Ok(judgement) => {
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision
                                    || this.current.as_ref().map(|item| item.unit_id)
                                        != Some(unit_id)
                                {
                                    return;
                                }
                                let next = AppState::global(cx)
                                    .scheduler
                                    .next_states(item.memory, item.days_elapsed(Utc::now()));
                                match next {
                                    Ok(next) => this.phase = Phase::Judged { judgement, next },
                                    Err(error) => {
                                        this.phase = Phase::Answering;
                                        this.error = Some(format!("FSRS 计算失败: {error}"));
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(error) => {
                            crate::diagnostics::error(
                                "review.judge.failed",
                                "AI judgement failed",
                                serde_json::json!({ "error": format!("{error:#}") }),
                            );
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision
                                    || this.current.as_ref().map(|item| item.unit_id)
                                        != Some(unit_id)
                                {
                                    return;
                                }
                                this.phase = Phase::Answering;
                                this.error = Some(format!(
                                    "判官调用失败: {error}\n{}",
                                    crate::diagnostics::log_hint()
                                ));
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
        let Some(item) = self.current.clone() else {
            return;
        };
        let Some(prompt) = self.prompt.clone() else {
            return;
        };
        let (judgement, next) = match &self.phase {
            Phase::Judged { judgement, next } => (judgement.clone(), next.clone()),
            _ => return,
        };
        let state = Scheduler::state_for(rating, &next);
        let due = Scheduler::due_date(state);
        let reps = item.reps + 1;
        let lapses = if rating == Rating::Again {
            item.lapses + 1
        } else {
            item.lapses
        };
        let memory = state.memory;
        let pool = AppState::global(cx).pool.clone();
        let user_answer = self.submitted_answer.clone();
        let feedback = judgement.feedback.clone();
        let retry_judgement = judgement.clone();
        let retry_next = next.clone();
        let answer_input = self.answer.clone();
        let window_handle = self.window;
        let completed_unit_id = item.unit_id;
        let revision = self.load_revision;
        self.phase = Phase::Scheduling;
        self.error = None;
        cx.notify();
        cx.spawn(
            move |this: gpui::WeakEntity<ReviewView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::dynamic_reviews::complete_review(
                            &pool,
                            &item,
                            prompt.id,
                            &user_answer,
                            &feedback,
                            rating,
                            memory,
                            due,
                            reps,
                            lapses,
                        )
                        .await
                    })
                    .await;
                    match result {
                        Ok(()) => {
                            cx.update_window(window_handle, move |_view, window, cx| {
                                answer_input
                                    .update(cx, |state, cx| state.set_value("", window, cx));
                            })
                            .ok();
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision {
                                    return;
                                }
                                this.queue
                                    .retain(|queued| queued.unit_id != completed_unit_id);
                                this.current = this.queue.first().cloned();
                                this.prompt = None;
                                this.selected_option = None;
                                this.show_source = false;
                                this.submitted_answer.clear();
                                if this.current.is_some() {
                                    this.prepare_current(cx);
                                } else {
                                    this.phase = Phase::Finished;
                                    cx.notify();
                                }
                            })
                            .ok();
                        }
                        Err(error) => {
                            this.update(&mut cx, |this, cx| {
                                if this.load_revision != revision {
                                    return;
                                }
                                this.phase = Phase::Judged {
                                    judgement: retry_judgement,
                                    next: retry_next,
                                };
                                this.error = Some(format!("保存复习记录失败: {error}"));
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let success_color = cx.theme().green;
        let warning_color = cx.theme().yellow;
        let error_color = cx.theme().red;
        let compact = window.viewport_size().width.as_f32() < 900.;
        let busy = matches!(
            self.phase,
            Phase::Loading | Phase::Judging | Phase::Scheduling
        );

        let header = page_header(
            RuizIcon::BrainCircuit,
            "复习",
            "每次根据知识单元和熟练度生成新题，再由 FSRS 安排下一次复习。",
            Some(
                Button::new("btn-reload-header")
                    .icon(IconName::Redo2)
                    .label("刷新队列")
                    .outline()
                    .loading(matches!(self.phase, Phase::Loading))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                    .into_any_element(),
            ),
            cx,
        );

        let scope_bar = v_flex()
            .w_full()
            .gap_2()
            .px_6()
            .py_3()
            .border_b_1()
            .border_color(colors.border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .text_xs()
                            .font_medium()
                            .text_color(colors.muted_foreground)
                            .child("复习分组"),
                    )
                    .child(
                        Combobox::new(&self.group_filter)
                            .small()
                            .cleanable(true)
                            .placeholder("全部分组")
                            .search_placeholder("搜索分组…")
                            .disabled(busy)
                            .w(px(320.))
                            .max_w_full(),
                    ),
            )
            .when(!self.selected_group_ids.is_empty(), |this| {
                let selected_group_ids = self.selected_group_ids.clone();
                this.child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(colors.muted_foreground)
                                .child("章节"),
                        )
                        .child(
                            Button::new("review-all-chapters")
                                .small()
                                .outline()
                                .disabled(busy)
                                .selected(self.selected_note_id.is_none())
                                .label("全部章节")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_note(None, cx);
                                })),
                        )
                        .children(
                            self.notes
                                .iter()
                                .filter(move |note| selected_group_ids.contains(&note.group_id))
                                .map(|note| {
                                    let note_id = note.id;
                                    Button::new(SharedString::from(format!(
                                        "review-note-{note_id}"
                                    )))
                                    .small()
                                    .outline()
                                    .disabled(busy)
                                    .selected(self.selected_note_id == Some(note_id))
                                    .label(note.title.clone())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_note(Some(note_id), cx);
                                    }))
                                }),
                        ),
                )
            });

        let body = match &self.phase {
            Phase::Loading => loading_state(colors).into_any_element(),
            Phase::Ready => empty_state(
                IconName::Play,
                "准备好开始复习了吗？",
                format!(
                    "当前范围有 {} 个到期知识单元，点击开始后逐题生成动态题目。",
                    self.queue.len()
                ),
                Some(
                    Button::new("btn-start-review")
                        .icon(IconName::Play)
                        .label("开始复习")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| this.start(cx)))
                        .into_any_element(),
                ),
                cx,
            )
            .into_any_element(),
            Phase::Finished => empty_state(
                IconName::CircleCheck,
                "当前范围的复习已经完成",
                "到期知识单元都处理完了，可以稍后回来或刷新队列。",
                Some(
                    Button::new("btn-reload")
                        .icon(IconName::Redo2)
                        .label("刷新队列")
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                        .into_any_element(),
                ),
                cx,
            )
            .into_any_element(),
            Phase::Judging => process_state(
                "AI 正在评估答案",
                "正在对比本次动态题面、稳定必答点和你的作答。",
                colors,
            )
            .into_any_element(),
            Phase::Scheduling => process_state(
                "正在安排下次复习",
                "正在保存题面快照、作答记录和知识单元的 FSRS 状态。",
                colors,
            )
            .into_any_element(),
            Phase::Answering | Phase::Judged { .. } => {
                let item = self.current.clone().expect("作答阶段必有知识单元");
                let prompt = self.prompt.clone().expect("作答阶段必有动态题面");
                let is_judged = matches!(self.phase, Phase::Judged { .. });
                let question = GroupBox::new()
                    .outline()
                    .title(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .flex_wrap()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::new(RuizIcon::BrainCircuit).size_4())
                                    .child(format!("{} · {}", item.note_title, item.topic)),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(status_badge(
                                        prompt.format.label(),
                                        colors.primary,
                                        colors,
                                    ))
                                    .child(status_badge(
                                        prompt.mastery.label(),
                                        success_color,
                                        colors,
                                    ))
                                    .child(status_badge(
                                        if prompt.generation_mode == "ai" {
                                            "AI 变式"
                                        } else {
                                            "备用题"
                                        },
                                        colors.muted_foreground,
                                        colors,
                                    ))
                                    .child(status_badge(
                                        &format!("剩余 {} 项", self.queue.len()),
                                        colors.muted_foreground,
                                        colors,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_medium()
                            .child(SharedString::from(prompt.question.clone())),
                    );

                let source = self.show_source.then(|| {
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
                                .child(div().text_sm().font_medium().child("原文证据"))
                                .child(
                                    div().text_sm().text_color(colors.muted_foreground).child(
                                        SharedString::from(
                                            prompt
                                                .source_excerpt
                                                .clone()
                                                .unwrap_or_else(|| "（没有原文引用）".into()),
                                        ),
                                    ),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("本题标准答案"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted_foreground)
                                        .child(SharedString::from(prompt.standard_answer.clone())),
                                ),
                        )
                });

                let response_control = if prompt.format == QuestionFormat::Choice {
                    v_flex()
                        .gap_2()
                        .children(prompt.options.iter().enumerate().map(|(index, option)| {
                            let option_value = option.clone();
                            Button::new(SharedString::from(format!("choice-option-{index}")))
                                .w_full()
                                .h_auto()
                                .min_h_8()
                                .py_2()
                                .outline()
                                .selected(self.selected_option.as_ref() == Some(option))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .items_start()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .font_medium()
                                                .child(format!("{}.", index + 1)),
                                        )
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .whitespace_normal()
                                                .text_left()
                                                .child(option.clone()),
                                        ),
                                )
                                .disabled(is_judged)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_option(option_value.clone(), cx);
                                }))
                        }))
                        .into_any_element()
                } else {
                    Input::new(&self.answer).h(px(180.)).into_any_element()
                };

                let answer_area = GroupBox::new()
                    .fill()
                    .title(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::SquareTerminal).size_4())
                            .child("你的回答"),
                    )
                    .child(response_control)
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .flex_wrap()
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
                                    .disabled(
                                        is_judged
                                            || (prompt.format == QuestionFormat::Choice
                                                && self.selected_option.is_none()),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                            ),
                    );

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
                                    .child(status_badge(
                                        &format!("{}/100", judgement.score),
                                        colors.primary,
                                        colors,
                                    )),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().font_medium().child(format!(
                                        "AI 建议：{}",
                                        rating_name(&judgement.rating)
                                    )))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(colors.muted_foreground)
                                            .child(SharedString::from(judgement.feedback.clone())),
                                    ),
                            )
                            .when(!judgement.point_results.is_empty(), |this| {
                                this.child(
                                    v_flex()
                                        .gap_2()
                                        .child(div().text_sm().font_medium().child("必答点检查"))
                                        .children(judgement.point_results.iter().map(|result| {
                                            let point = prompt
                                                .required_points
                                                .get(result.point_index)
                                                .cloned()
                                                .unwrap_or_else(|| "未知必答点".into());
                                            let (label, color) = match result.status.as_str() {
                                                "correct" => ("已掌握", success_color),
                                                "partial" => ("部分掌握", warning_color),
                                                "missing" => ("未回答", warning_color),
                                                _ => ("有误", error_color),
                                            };
                                            h_flex()
                                                .items_start()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .mt_1()
                                                        .size_2()
                                                        .flex_shrink_0()
                                                        .rounded_full()
                                                        .bg(color),
                                                )
                                                .child(
                                                    v_flex()
                                                        .min_w_0()
                                                        .gap_0p5()
                                                        .child(
                                                            h_flex()
                                                                .gap_2()
                                                                .flex_wrap()
                                                                .child(div().text_sm().child(point))
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .font_medium()
                                                                        .text_color(color)
                                                                        .child(label),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(colors.muted_foreground)
                                                                .child(SharedString::from(
                                                                    result.feedback.clone(),
                                                                )),
                                                        ),
                                                )
                                        })),
                                )
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(rating_button(
                                        cx,
                                        Rating::Again,
                                        format!("重来 · {}", interval_label(next.again.interval)),
                                        judgement.rating == "again",
                                    ))
                                    .child(rating_button(
                                        cx,
                                        Rating::Hard,
                                        format!("困难 · {}", interval_label(next.hard.interval)),
                                        judgement.rating == "hard",
                                    ))
                                    .child(rating_button(
                                        cx,
                                        Rating::Good,
                                        format!("良好 · {}", interval_label(next.good.interval)),
                                        judgement.rating == "good",
                                    ))
                                    .child(rating_button(
                                        cx,
                                        Rating::Easy,
                                        format!("简单 · {}", interval_label(next.easy.interval)),
                                        judgement.rating == "easy",
                                    )),
                            ),
                    )
                } else {
                    None
                };

                v_flex()
                    .w_full()
                    .max_w(px(880.))
                    .mx_auto()
                    .when(compact, |this| this.p_4())
                    .when(!compact, |this| this.p_6())
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
                    .children(source)
                    .child(answer_area)
                    .children(judge_area)
                    .into_any_element()
            }
        };

        let error = self.error.clone().map(|message| {
            Alert::error("review-alert", message)
                .banner()
                .on_close(cx.listener(|this, _, _, cx| {
                    this.error = None;
                    cx.notify();
                }))
        });
        let notice = self.generation_notice.clone().map(|message| {
            Alert::warning("generation-notice", message)
                .banner()
                .on_close(cx.listener(|this, _, _, cx| {
                    this.generation_notice = None;
                    cx.notify();
                }))
        });

        v_flex()
            .size_full()
            .child(header)
            .child(scope_bar)
            .children(error)
            .children(notice)
            .child(
                div()
                    .id("review-scroll-wrap")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
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

async fn prepare_prompt(
    pool: sqlx::SqlitePool,
    ai: Option<ChatClient>,
    item: ReviewItem,
    adaptive_answer_formats: bool,
) -> anyhow::Result<PreparedPrompt> {
    let recent = db::dynamic_reviews::recent_questions(&pool, item.unit_id, 5).await?;
    let format = item.question_format(adaptive_answer_formats);
    let (mut prompt, notice) = match ai {
        Some(ai) => {
            match crate::ai::dynamic_question::generate(&ai, &item, format, &recent).await {
                Ok(prompt) => (prompt, None),
                Err(error) => {
                    crate::diagnostics::warn(
                        "review.question.fallback",
                        "Dynamic question generation failed; using seed prompt",
                        serde_json::json!({
                            "unit_id": item.unit_id,
                            "error": format!("{error:#}"),
                        }),
                    );
                    (
                        item.fallback_prompt(),
                        Some(format!("动态出题失败，已使用备用题: {error}")),
                    )
                }
            }
        }
        None => (
            item.fallback_prompt(),
            Some("未配置 AI，当前使用备用题；提交判分前需要配置 AI".into()),
        ),
    };
    let prompt_id = db::dynamic_reviews::insert_prompt(&pool, &prompt).await?;
    prompt.id = Some(prompt_id);
    Ok(PreparedPrompt {
        prompt,
        notice,
        adaptive_answer_formats,
    })
}

fn loading_state(colors: ThemeColor) -> impl IntoElement {
    v_flex()
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
                        .child(Spinner::new().small().color(colors.primary))
                        .child("正在准备动态题目"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted_foreground)
                        .child("正在读取知识证据、熟练度和最近题目，生成本次复习题面。"),
                )
                .child(Skeleton::new().w_full())
                .child(Skeleton::new().secondary().w_3_5()),
        )
}

fn process_state(
    title: &'static str,
    description: &'static str,
    colors: ThemeColor,
) -> impl IntoElement {
    v_flex()
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
                        .child(Spinner::new().small().color(colors.primary))
                        .child(title),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted_foreground)
                        .child(description),
                )
                .child(Skeleton::new().secondary().w_3_5()),
        )
}

fn status_badge(label: &str, color: gpui::Hsla, colors: ThemeColor) -> impl IntoElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(if color == colors.muted_foreground {
            colors.muted
        } else {
            color.opacity(0.12)
        })
        .text_xs()
        .text_color(color)
        .child(label.to_string())
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

fn rating_name(rating: &str) -> &'static str {
    match rating {
        "again" => "重来",
        "hard" => "困难",
        "good" => "良好",
        "easy" => "简单",
        _ => "由你决定",
    }
}

fn rating_button(
    cx: &mut Context<ReviewView>,
    rating: Rating,
    label: impl Into<SharedString>,
    recommended: bool,
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
    let icon = if recommended {
        Icon::new(RuizIcon::Sparkles)
    } else {
        Icon::new(icon)
    };
    let mut button = Button::new(id).icon(icon).label(label);
    button = match rating {
        Rating::Again => button.danger(),
        Rating::Hard => button.warning(),
        Rating::Good => button.success(),
        Rating::Easy => button.primary(),
    };
    if !recommended {
        button = button.outline();
    }
    button.on_click(cx.listener(move |this, _, _, cx| this.rate(rating, cx)))
}
