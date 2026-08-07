use gpui::{
    AnyWindowHandle, Context, Entity, EventEmitter, IntoElement, Render, ScrollHandle,
    SharedString, Window, div, point, prelude::*, px, relative,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    notification::Notification,
    scroll::ScrollableElement as _,
    spinner::Spinner,
    text::TextView,
    v_flex,
};

use crate::{
    ai,
    assets::RuizIcon,
    db,
    domain::{
        dynamic_review::QuestionFormat,
        knowledge::{KnowledgeUnit, MaterialAnalysis},
        learning::{
            ContentBlock, LearningPlan, LearningPrompt, LearningSession, LearningStep,
            LearningStepKind, checkpoint_question_targets, content_hash, fallback_plan,
            map_unit_sources, parse_content_blocks, validate_plan,
        },
        note::Note,
    },
    settings::{AppSettings, save_config},
    state::AppState,
    ui::notifications,
};

pub struct LearningView {
    note: Note,
    analysis: Option<MaterialAnalysis>,
    units: Vec<KnowledgeUnit>,
    plan: Option<LearningPlan>,
    blocks: Vec<ContentBlock>,
    session: Option<LearningSession>,
    current_step: usize,
    unlocked_step: usize,
    prompts: Vec<LearningPrompt>,
    current_prompt_index: usize,
    retry_prompt: Option<LearningPrompt>,
    recap_unit_ids: Vec<String>,
    answer: Entity<InputState>,
    selected_choice: Option<String>,
    first_result: Option<String>,
    feedback: Option<String>,
    remediation: bool,
    checkpoint_assisted: bool,
    awaiting_continue: bool,
    loading: bool,
    loading_prompt: bool,
    prefetching_prompt_step_id: Option<i64>,
    busy: bool,
    load_error: Option<String>,
    outline_collapsed: bool,
    window: AnyWindowHandle,
    content_scroll: ScrollHandle,
    outline_scroll: ScrollHandle,
}

pub enum LearningViewEvent {
    Exit,
}

impl EventEmitter<LearningViewEvent> for LearningView {}

impl LearningView {
    pub fn new(
        note: Note,
        analysis: Option<MaterialAnalysis>,
        units: Vec<KnowledgeUnit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let outline_collapsed = AppSettings::global(cx)
            .settings
            .ui
            .learning_outline_collapsed;
        let answer = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("写下你的回答…")
                .multi_line(true)
                .rows(5)
        });
        let mut view = Self {
            note,
            analysis,
            units,
            plan: None,
            blocks: Vec::new(),
            session: None,
            current_step: 0,
            unlocked_step: 0,
            prompts: Vec::new(),
            current_prompt_index: 0,
            retry_prompt: None,
            recap_unit_ids: Vec::new(),
            answer,
            selected_choice: None,
            first_result: None,
            feedback: None,
            remediation: false,
            checkpoint_assisted: false,
            awaiting_continue: false,
            loading: true,
            loading_prompt: false,
            prefetching_prompt_step_id: None,
            busy: false,
            load_error: None,
            outline_collapsed,
            window: window.window_handle(),
            content_scroll: ScrollHandle::new(),
            outline_scroll: ScrollHandle::new(),
        };
        view.load(cx);
        view
    }

    fn notify(&self, notification: Notification, cx: &mut Context<Self>) {
        notifications::push(self.window, notification, cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let pool = AppState::global(cx).pool.clone();
        let ai_client = AppState::global(cx).ai.clone();
        let note = self.note.clone();
        let analysis = self.analysis.clone();
        let units = self.units.clone();
        cx.spawn(
            move |this: gpui::WeakEntity<LearningView>, cx: &mut gpui::AsyncApp| {
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let hash = content_hash(&note.content);
                        let plan = if let Some(plan) =
                            db::learning::latest_valid_plan(&pool, note.id, &hash).await?
                        {
                            plan
                        } else {
                            let blocks = parse_content_blocks(note.id, &note.content);
                            if blocks.is_empty() {
                                anyhow::bail!("笔记正文为空，无法开始学习");
                            }
                            let links = map_unit_sources(&units, &blocks);
                            let generated = if let Some(client) = ai_client.as_ref() {
                                ai::learning_plan::generate(
                                    client,
                                    &note,
                                    analysis.as_ref(),
                                    &blocks,
                                    &units,
                                    &links,
                                )
                                .await
                                .ok()
                            } else {
                                None
                            };
                            let plan = generated.unwrap_or_else(|| {
                                fallback_plan(note.id, &note.title, &hash, &blocks, &units, &links)
                            });
                            validate_plan(&plan, &blocks, &units)?;
                            db::learning::save_plan(&pool, &blocks, &links, &plan).await?;
                            db::learning::latest_valid_plan(&pool, note.id, &hash)
                                .await?
                                .ok_or_else(|| anyhow::anyhow!("学习路线保存失败"))?
                        };
                        let blocks = db::learning::blocks_for_plan(&pool, &plan).await?;
                        let session = db::learning::resume_or_start_session(
                            &pool,
                            plan.id.ok_or_else(|| anyhow::anyhow!("学习路线缺少 ID"))?,
                        )
                        .await?;
                        anyhow::Ok((plan, blocks, session))
                    })
                    .await;
                    this.update(&mut cx, |this, cx| {
                        match result {
                            Ok((plan, blocks, session)) => {
                                this.unlocked_step =
                                    session.current_step_index.min(plan.steps.len());
                                this.current_step = this.unlocked_step;
                                this.plan = Some(plan);
                                this.blocks = blocks;
                                this.session = Some(session);
                                this.loading = false;
                                this.prepare_current(cx);
                            }
                            Err(error) => {
                                this.loading = false;
                                this.load_error = Some(format!("准备学习路线失败: {error:#}"));
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    fn current(&self) -> Option<&LearningStep> {
        self.plan.as_ref()?.steps.get(self.current_step)
    }

    fn is_reviewing(&self) -> bool {
        self.current_step != self.unlocked_step
    }

    fn select_step(&mut self, step_index: usize, cx: &mut Context<Self>) {
        let total = self.plan.as_ref().map_or(0, |plan| plan.steps.len());
        if self.busy
            || self.loading_prompt
            || step_index == self.current_step
            || step_index >= total
            || step_index > self.unlocked_step
        {
            return;
        }
        self.current_step = step_index;
        self.prepare_current(cx);
        cx.notify();
    }

    fn return_to_progress(&mut self, cx: &mut Context<Self>) {
        self.current_step = self.unlocked_step;
        self.prepare_current(cx);
        cx.notify();
    }

    fn toggle_outline(&mut self, cx: &mut Context<Self>) {
        self.outline_collapsed = !self.outline_collapsed;
        let config = {
            let settings = AppSettings::global_mut(cx);
            settings.settings.ui.learning_outline_collapsed = self.outline_collapsed;
            settings.settings.clone()
        };
        if let Err(error) = save_config(&config) {
            crate::diagnostics::error(
                "learning.outline.save_failed",
                "Failed to persist learning outline state",
                serde_json::json!({ "error": error.to_string() }),
            );
        }
        cx.notify();
    }

    fn prepare_current(&mut self, cx: &mut Context<Self>) {
        self.content_scroll.set_offset(point(px(0.), px(0.)));
        self.prompts.clear();
        self.current_prompt_index = 0;
        self.retry_prompt = None;
        self.recap_unit_ids.clear();
        self.selected_choice = None;
        self.first_result = None;
        self.feedback = None;
        self.remediation = false;
        self.checkpoint_assisted = false;
        self.awaiting_continue = false;
        self.loading_prompt = false;
        let Some(step) = self.current().cloned() else {
            return;
        };
        if self.is_reviewing() {
            if step.kind == LearningStepKind::Checkpoint {
                self.prepare_prompt(step, true, false, cx);
            }
            return;
        }
        if step.kind == LearningStepKind::Recap {
            self.prepare_recap(step, cx);
            return;
        }
        if step.kind == LearningStepKind::Read {
            self.prefetch_next_checkpoint(cx);
            return;
        }
        if step.kind != LearningStepKind::Checkpoint {
            return;
        }
        let Some(step_id) = step.id else {
            self.notify(Notification::error("理解检查尚未保存"), cx);
            return;
        };
        if self.prefetching_prompt_step_id == Some(step_id) {
            self.loading_prompt = true;
            return;
        }
        self.prepare_prompt(step, true, true, cx);
    }

    fn prefetch_next_checkpoint(&mut self, cx: &mut Context<Self>) {
        let next = self.plan.as_ref().and_then(|plan| {
            plan.steps
                .iter()
                .skip(self.current_step + 1)
                .find(|step| step.kind == LearningStepKind::Checkpoint)
                .cloned()
        });
        let Some(step) = next else {
            return;
        };
        let Some(step_id) = step.id else {
            return;
        };
        if self.prefetching_prompt_step_id == Some(step_id) {
            return;
        }
        self.prepare_prompt(step, false, true, cx);
    }

    fn prepare_prompt(
        &mut self,
        step: LearningStep,
        foreground: bool,
        generate_missing: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(step_id) = step.id else {
            if foreground {
                self.notify(Notification::error("理解检查尚未保存"), cx);
            }
            return;
        };
        let pool = AppState::global(cx).pool.clone();
        let client = AppState::global(cx).ai.clone();
        let units = self.units.clone();
        let blocks = self.blocks.clone();
        self.prefetching_prompt_step_id = Some(step_id);
        if foreground {
            self.loading_prompt = true;
        }
        cx.spawn(
            move |this: gpui::WeakEntity<LearningView>, cx: &mut gpui::AsyncApp| {
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let targets = checkpoint_question_targets(&step, &units);
                        let total_questions = targets.len();
                        let mut prompts = db::learning::prompts_for_step(&pool, step_id).await?;
                        prompts.sort_by_key(|prompt| prompt.position);
                        prompts.dedup_by_key(|prompt| prompt.position);
                        if generate_missing {
                            for (position, target_unit_ids) in targets.iter().enumerate() {
                                if prompts.iter().any(|prompt| prompt.position == position) {
                                    continue;
                                }
                                let recent_questions = prompts
                                    .iter()
                                    .map(|prompt| prompt.question.clone())
                                    .collect::<Vec<_>>();
                                let generated = if let Some(client) = client.as_ref() {
                                    ai::learning_question::generate(
                                        client,
                                        &step,
                                        &units,
                                        &blocks,
                                        ai::learning_question::QuestionContext {
                                            target_unit_ids,
                                            position,
                                            total_questions,
                                            recent_questions: &recent_questions,
                                        },
                                    )
                                    .await
                                    .ok()
                                } else {
                                    None
                                };
                                let prompt = match generated {
                                    Some(prompt) => prompt,
                                    None => {
                                        db::learning::fallback_prompt(
                                            &pool,
                                            &step,
                                            &units,
                                            target_unit_ids,
                                            position,
                                        )
                                        .await?
                                    }
                                };
                                prompts.push(db::learning::insert_prompt(&pool, &prompt).await?);
                                prompts.sort_by_key(|prompt| prompt.position);
                            }
                        }
                        prompts.retain(|prompt| prompt.position < total_questions);
                        anyhow::Ok(prompts)
                    })
                    .await;
                    this.update(&mut cx, |this, cx| {
                        if this.prefetching_prompt_step_id == Some(step_id) {
                            this.prefetching_prompt_step_id = None;
                        }
                        let is_current = this.current().and_then(|step| step.id) == Some(step_id);
                        if is_current {
                            this.loading_prompt = false;
                            match result {
                                Ok(prompts) => {
                                    this.prompts = prompts;
                                    this.current_prompt_index = 0;
                                }
                                Err(error) => this.notify(
                                    Notification::error(format!("准备理解检查失败: {error:#}")),
                                    cx,
                                ),
                            }
                        } else if let Err(error) = result {
                            crate::diagnostics::warn(
                                "learning.prompt.prefetch_failed",
                                "Learning prompt prefetch failed",
                                serde_json::json!({
                                    "learning_step_id": step_id,
                                    "error": format!("{error:#}"),
                                }),
                            );
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    fn prepare_recap(&mut self, step: LearningStep, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if step.id.is_none() {
            self.notify(Notification::error("主题回顾尚未保存"), cx);
            return;
        }
        let pool = AppState::global(cx).pool.clone();
        self.busy = true;
        cx.spawn(
            move |this: gpui::WeakEntity<LearningView>, cx: &mut gpui::AsyncApp| {
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let candidates =
                            db::learning::recap_candidates(&pool, session.id, &step.unit_ids)
                                .await?;
                        if candidates.is_empty() {
                            let next =
                                db::learning::complete_step(&pool, &session, &step, None, false)
                                    .await?;
                            anyhow::Ok((candidates, Some(next)))
                        } else {
                            anyhow::Ok((candidates, None))
                        }
                    })
                    .await;
                    this.update(&mut cx, |this, cx| {
                        this.busy = false;
                        match result {
                            Ok((candidates, next)) => {
                                this.recap_unit_ids = candidates;
                                if let Some(next) = next {
                                    this.current_step = next;
                                    this.unlocked_step = next;
                                    if let Some(session) = this.session.as_mut() {
                                        session.current_step_index = next;
                                    }
                                    this.prepare_current(cx);
                                }
                            }
                            Err(error) => {
                                this.notify(
                                    Notification::error(format!("准备主题回顾失败: {error:#}")),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    fn complete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_reviewing() {
            return;
        }
        let (Some(session), Some(step)) = (self.session.clone(), self.current().cloned()) else {
            return;
        };
        let pool = AppState::global(cx).pool.clone();
        let result = self.first_result.clone();
        let assisted = self.checkpoint_assisted;
        self.busy = true;
        self.answer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.spawn(
            move |this: gpui::WeakEntity<LearningView>, cx: &mut gpui::AsyncApp| {
                let mut cx = (*cx).clone();
                async move {
                    let completed = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        db::learning::complete_step(
                            &pool,
                            &session,
                            &step,
                            result.as_deref(),
                            assisted,
                        )
                        .await
                    })
                    .await;
                    this.update(&mut cx, |this, cx| {
                        this.busy = false;
                        match completed {
                            Ok(next) => {
                                this.current_step = next;
                                this.unlocked_step = next;
                                if let Some(session) = this.session.as_mut() {
                                    session.current_step_index = next;
                                }
                                this.prepare_current(cx);
                            }
                            Err(error) => this.notify(
                                Notification::error(format!("保存学习进度失败: {error:#}")),
                                cx,
                            ),
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    fn submit_answer(&mut self, cx: &mut Context<Self>) {
        if self.is_reviewing() {
            return;
        }
        let Some(step) = self.current().cloned() else {
            return;
        };
        let Some(prompt) = self
            .retry_prompt
            .clone()
            .or_else(|| self.prompts.get(self.current_prompt_index).cloned())
        else {
            return;
        };
        let answer = if prompt.format == QuestionFormat::Choice {
            self.selected_choice.clone().unwrap_or_default()
        } else {
            self.answer.read(cx).value().trim().to_string()
        };
        if answer.is_empty() {
            self.notify(Notification::warning("请先作答，或选择“不会”"), cx);
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(step_id) = step.id else {
            return;
        };
        let pool = AppState::global(cx).pool.clone();
        let client = AppState::global(cx).ai.clone();
        let assisted = self.remediation;
        self.busy = true;
        cx.spawn(
            move |this: gpui::WeakEntity<LearningView>, cx: &mut gpui::AsyncApp| {
                let mut cx = (*cx).clone();
                async move {
                    let judged = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let (score, mut feedback) = if prompt.format == QuestionFormat::Choice {
                            let correct = answer.trim() == prompt.standard_answer.trim();
                            (
                                if correct { 100 } else { 0 },
                                if correct {
                                    "回答正确，已经抓住这个关键点。".to_string()
                                } else {
                                    "这个选择还不准确。".to_string()
                                },
                            )
                        } else if let Some(client) = client.as_ref() {
                            let judgement = ai::judge::judge(
                                client,
                                &prompt.question,
                                &prompt.standard_answer,
                                &prompt.required_points,
                                &answer,
                            )
                            .await?;
                            (judgement.score, judgement.feedback)
                        } else {
                            let correct = normalize(&answer) == normalize(&prompt.standard_answer);
                            (
                                if correct { 100 } else { 0 },
                                if correct {
                                    "回答正确。".into()
                                } else {
                                    "离线状态下只能进行精确答案比较。".into()
                                },
                            )
                        };
                        let result = if score >= 75 {
                            "correct"
                        } else if score >= 50 {
                            "partial"
                        } else {
                            "incorrect"
                        };
                        if assisted {
                            feedback.push_str(&format!("\n\n参考答案：{}", prompt.standard_answer));
                        }
                        db::learning::save_attempt(
                            &pool,
                            session.id,
                            step_id,
                            prompt.id,
                            &prompt.unit_ids,
                            if assisted { 2 } else { 1 },
                            &answer,
                            result,
                            Some(score),
                            &feedback,
                            assisted,
                        )
                        .await?;
                        anyhow::Ok((result.to_string(), feedback))
                    })
                    .await;
                    this.update(&mut cx, |this, cx| {
                        this.busy = false;
                        match judged {
                            Ok((result, feedback)) => {
                                if !this.remediation {
                                    merge_result(&mut this.first_result, &result);
                                }
                                this.feedback = Some(feedback);
                                if result == "correct" || assisted {
                                    this.awaiting_continue = true;
                                } else {
                                    this.begin_remediation();
                                }
                            }
                            Err(error) => this.notify(
                                Notification::error(format!(
                                    "判分失败，回答已留在输入框中: {error:#}"
                                )),
                                cx,
                            ),
                        }
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    fn begin_remediation(&mut self) {
        self.remediation = true;
        self.checkpoint_assisted = true;
        if let Some(prompt) = self.prompts.get(self.current_prompt_index) {
            self.retry_prompt = Some(remediation_prompt(prompt));
        }
    }

    fn dont_know(&mut self, cx: &mut Context<Self>) {
        if self.is_reviewing() {
            return;
        }
        merge_result(&mut self.first_result, "incorrect");
        self.feedback = Some("先回看下方原文证据，再用自己的话重新回答原题。".into());
        self.begin_remediation();
        cx.notify();
    }

    fn continue_checkpoint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_reviewing() {
            return;
        }
        if self.current_prompt_index + 1 >= self.prompts.len() {
            self.complete_current(window, cx);
            return;
        }
        self.current_prompt_index += 1;
        self.retry_prompt = None;
        self.selected_choice = None;
        self.feedback = None;
        self.remediation = false;
        self.awaiting_continue = false;
        self.answer
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.content_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session.clone() {
            let pool = AppState::global(cx).pool.clone();
            gpui_tokio::Tokio::spawn(cx, async move {
                let _ = db::learning::pause_session(&pool, session.id).await;
            })
            .detach();
        }
        cx.emit(LearningViewEvent::Exit);
    }
}

impl Render for LearningView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let total = self.plan.as_ref().map(|plan| plan.steps.len()).unwrap_or(0);
        let completed = self.unlocked_step.min(total);
        let progress = if total == 0 {
            0.0
        } else {
            completed as f32 / total as f32
        };

        let top = h_flex()
            .h(px(64.))
            .flex_shrink_0()
            .px_5()
            .items_center()
            .gap_4()
            .border_b_1()
            .border_color(colors.border)
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
                    .child(
                        div()
                            .truncate()
                            .font_semibold()
                            .child(self.note.title.clone()),
                    )
                    .child(div().text_xs().text_color(colors.muted_foreground).child(
                        if self.loading {
                            "正在准备学习路线…".into()
                        } else if completed >= total && total > 0 {
                            "本章学习已完成".into()
                        } else {
                            format!("进度 {completed}/{total}")
                        },
                    )),
            )
            .child(
                div()
                    .w(px(180.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(colors.muted)
                    .child(
                        div()
                            .h_full()
                            .w(relative(progress))
                            .rounded_full()
                            .bg(colors.primary),
                    ),
            )
            .child(
                Button::new("pause-learning")
                    .icon(IconName::Pause)
                    .label("暂停")
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| this.pause(cx))),
            );

        let main_content = if self.loading {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child("正在解析正文并编排学习路线…")
                .into_any_element()
        } else if let Some(error) = self.load_error.clone() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    Icon::new(IconName::CircleX)
                        .size_8()
                        .text_color(colors.danger),
                )
                .child(div().max_w(px(620.)).text_sm().child(error))
                .into_any_element()
        } else if completed >= total && !self.is_reviewing() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    Icon::new(RuizIcon::GraduationCap)
                        .size_8()
                        .text_color(colors.primary),
                )
                .child(div().text_xl().font_semibold().child("本章学习完成"))
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted_foreground)
                        .child("理解检查已经保存，相关知识点会按结果进入正式复习。"),
                )
                .child(
                    Button::new("close-completed")
                        .label("返回笔记")
                        .primary()
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(LearningViewEvent::Exit);
                        })),
                )
                .into_any_element()
        } else {
            self.render_step(cx).into_any_element()
        };

        let body = h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .h_full()
                    .child(main_content),
            )
            .child(
                div()
                    .w(if self.outline_collapsed {
                        px(52.)
                    } else {
                        px(340.)
                    })
                    .h_full()
                    .flex_shrink_0()
                    .border_l_1()
                    .border_color(colors.border)
                    .child(self.render_outline(cx)),
            );
        v_flex()
            .size_full()
            .bg(colors.background)
            .child(top)
            .child(body)
    }
}

impl LearningView {
    fn render_step(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let step = self.current().cloned().expect("current step");
        let reviewing = self.is_reviewing();
        let content = match step.kind {
            LearningStepKind::Read => {
                let blocks = step
                    .block_ids
                    .iter()
                    .filter_map(|id| self.blocks.iter().find(|block| &block.local_id == id))
                    .cloned()
                    .collect::<Vec<_>>();
                v_flex()
                    .gap_5()
                    .children(blocks.into_iter().map(|block| {
                        div().text_color(colors.foreground).child(
                            TextView::markdown(
                                format!("learning-source-{}", block.local_id),
                                block.source_text,
                            )
                            .selectable(true),
                        )
                    }))
                    .child(if reviewing {
                        self.review_return_button(cx)
                    } else {
                        h_flex()
                            .justify_end()
                            .pt_3()
                            .child(
                                Button::new("complete-read")
                                    .icon(IconName::ArrowRight)
                                    .label("继续")
                                    .primary()
                                    .loading(self.busy)
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.complete_current(window, cx);
                                    })),
                            )
                            .into_any_element()
                    })
                    .into_any_element()
            }
            LearningStepKind::Checkpoint => self.render_checkpoint(cx).into_any_element(),
            LearningStepKind::Recap => {
                let recap_unit_ids = if reviewing {
                    step.unit_ids.clone()
                } else {
                    self.recap_unit_ids.clone()
                };
                v_flex()
                    .gap_4()
                    .child(div().text_sm().text_color(colors.muted_foreground).child(
                        if reviewing {
                            "本次主题回顾涉及以下学习目标。"
                        } else {
                            "回想这个主题中刚刚遇到的疑问和易错点，再继续下一部分。"
                        },
                    ))
                    .children(recap_unit_ids.iter().filter_map(|id| {
                        self.units
                            .iter()
                            .find(|unit| &unit.local_id == id)
                            .map(|unit| {
                                h_flex()
                                    .items_start()
                                    .gap_2()
                                    .child(
                                        Icon::new(IconName::CircleCheck)
                                            .size_4()
                                            .text_color(colors.primary),
                                    )
                                    .child(
                                        TextView::markdown(
                                            format!("learning-recap-objective-{}", unit.local_id),
                                            unit.objective.clone(),
                                        )
                                        .selectable(true),
                                    )
                            })
                    }))
                    .child(if reviewing {
                        self.review_return_button(cx)
                    } else {
                        h_flex()
                            .justify_end()
                            .child(
                                Button::new("complete-recap")
                                    .label("完成回顾")
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.complete_current(window, cx);
                                    })),
                            )
                            .into_any_element()
                    })
                    .into_any_element()
            }
        };
        div()
            .id("learning-content-scroll-wrap")
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .id("learning-content-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.content_scroll)
                    .child(
                        v_flex()
                            .w(relative(0.9))
                            .max_w(px(760.))
                            .mx_auto()
                            .py_8()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(colors.primary)
                                    .child(step.topic_title),
                            )
                            .child(div().text_xl().font_semibold().mb_4().child(format!(
                                "{}{}",
                                match step.kind {
                                    LearningStepKind::Read => "阅读原文",
                                    LearningStepKind::Checkpoint => "检查理解",
                                    LearningStepKind::Recap => "主题回顾",
                                },
                                if reviewing { " · 回看" } else { "" }
                            )))
                            .child(content),
                    ),
            )
            .vertical_scrollbar(&self.content_scroll)
    }

    fn render_checkpoint(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        if self.loading_prompt {
            return v_flex()
                .items_center()
                .gap_2()
                .child(Spinner::new().small())
                .child("正在准备问题…")
                .into_any_element();
        }
        if self.is_reviewing() {
            let questions = if self.prompts.is_empty() {
                v_flex()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.muted_foreground)
                            .child("这次理解检查没有可回看的题目。"),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .gap_5()
                    .children(self.prompts.iter().enumerate().map(|(index, prompt)| {
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(colors.muted_foreground)
                                    .child(format!(
                                        "第 {} 题 · {}",
                                        index + 1,
                                        prompt.format.label()
                                    )),
                            )
                            .child(
                                TextView::markdown(
                                    ("learning-review-question", index),
                                    prompt.question.clone(),
                                )
                                .selectable(true)
                                .font_medium(),
                            )
                            .child(
                                div()
                                    .p_3()
                                    .rounded_md()
                                    .bg(colors.success.opacity(0.08))
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_medium()
                                                    .text_color(colors.success)
                                                    .child("参考答案"),
                                            )
                                            .child(
                                                TextView::markdown(
                                                    ("learning-review-answer", index),
                                                    prompt.standard_answer.clone(),
                                                )
                                                .selectable(true),
                                            ),
                                    ),
                            )
                    }))
                    .into_any_element()
            };
            return v_flex()
                .gap_5()
                .child(questions)
                .child(self.review_return_button(cx))
                .into_any_element();
        }
        let Some(prompt) = self
            .retry_prompt
            .as_ref()
            .or_else(|| self.prompts.get(self.current_prompt_index))
        else {
            return div().child("问题暂不可用").into_any_element();
        };
        let prompt_format = prompt.format;
        let selected = self.selected_choice.clone();
        let mut view = v_flex()
            .gap_4()
            .child(
                div()
                    .text_xs()
                    .font_medium()
                    .text_color(colors.muted_foreground)
                    .child(format!(
                        "第 {}/{} 题 · {}",
                        self.current_prompt_index + 1,
                        self.prompts.len(),
                        prompt.format.label()
                    )),
            )
            .child(
                div().text_base().font_medium().child(
                    TextView::markdown("learning-current-question", prompt.question.clone())
                        .selectable(true),
                ),
            );
        if self.remediation {
            let evidence = self
                .current()
                .into_iter()
                .flat_map(|step| &step.source_step_ids)
                .filter_map(|source_id| {
                    self.plan
                        .as_ref()?
                        .steps
                        .iter()
                        .find(|step| &step.local_id == source_id)
                })
                .flat_map(|step| &step.block_ids)
                .filter_map(|id| self.blocks.iter().find(|block| &block.local_id == id))
                .map(|block| block.source_text.clone())
                .collect::<Vec<_>>()
                .join("\n\n");
            view = view.child(
                div()
                    .p_3()
                    .border_l_2()
                    .border_color(colors.warning)
                    .bg(colors.warning.opacity(0.08))
                    .child(
                        TextView::markdown("learning-remediation-evidence", evidence)
                            .selectable(true),
                    ),
            );
        }
        if prompt_format == QuestionFormat::Choice {
            view = view.children(prompt.options.iter().enumerate().map(|(index, option)| {
                let value = option.clone();
                Button::new(SharedString::from(format!("learning-option-{index}")))
                    .w_full()
                    .h_auto()
                    .min_h_8()
                    .py_2()
                    .outline()
                    .selected(selected.as_ref() == Some(option))
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_choice = Some(value.clone());
                        cx.notify();
                    }))
            }));
        } else {
            view = view.child(Input::new(&self.answer).h(px(140.)));
        }
        if let Some(feedback) = self.feedback.clone() {
            view = view.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(if self.awaiting_continue {
                        colors.success.opacity(0.1)
                    } else {
                        colors.warning.opacity(0.1)
                    })
                    .child(
                        TextView::markdown("learning-checkpoint-feedback", feedback)
                            .selectable(true),
                    ),
            );
        }
        view.child(
            h_flex()
                .justify_between()
                .gap_2()
                .child(
                    Button::new("learning-dont-know")
                        .label("不会")
                        .secondary()
                        .disabled(self.busy || self.awaiting_continue || self.remediation)
                        .on_click(cx.listener(|this, _, _, cx| this.dont_know(cx))),
                )
                .child(if self.awaiting_continue {
                    Button::new("learning-next")
                        .label("继续")
                        .icon(IconName::ArrowRight)
                        .primary()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.continue_checkpoint(window, cx)),
                        )
                } else {
                    Button::new("learning-submit")
                        .label(if self.remediation {
                            "提交补救回答"
                        } else {
                            "提交回答"
                        })
                        .primary()
                        .loading(self.busy)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.submit_answer(cx)))
                }),
        )
        .into_any_element()
    }

    fn review_return_button(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let total = self.plan.as_ref().map_or(0, |plan| plan.steps.len());
        h_flex()
            .justify_end()
            .pt_3()
            .child(
                Button::new("return-to-learning-progress")
                    .icon(IconName::ArrowRight)
                    .label(if self.unlocked_step >= total {
                        "返回完成页"
                    } else {
                        "返回当前进度"
                    })
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| this.return_to_progress(cx))),
            )
            .into_any_element()
    }

    fn render_outline_item(
        &self,
        step: &LearningStep,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.theme().colors;
        let step_index = step.position;
        let selected = step_index == self.current_step;
        let completed = step_index < self.unlocked_step;
        let clickable = step_index != self.current_step
            && step_index <= self.unlocked_step
            && !self.busy
            && !self.loading_prompt;
        let locked = step_index > self.unlocked_step;
        let label = format!(
            "{} · {}",
            step.topic_title,
            match step.kind {
                LearningStepKind::Read => "阅读",
                LearningStepKind::Checkpoint => "检查",
                LearningStepKind::Recap => "回顾",
            }
        );
        let icon = match step.kind {
            LearningStepKind::Read => Icon::new(IconName::BookOpen),
            LearningStepKind::Checkpoint => Icon::new(RuizIcon::Pencil),
            LearningStepKind::Recap => Icon::new(RuizIcon::BrainCircuit),
        }
        .size_4()
        .text_color(if completed {
            colors.success
        } else if selected {
            colors.primary
        } else {
            colors.muted_foreground
        });

        if collapsed {
            return Button::new(SharedString::from(format!(
                "learning-outline-step-{step_index}"
            )))
            .small()
            .ghost()
            .selected(selected)
            .icon(icon)
            .tooltip(label)
            .disabled(locked || self.busy || self.loading_prompt)
            .when(clickable, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| {
                    this.select_step(step_index, cx);
                }))
            })
            .into_any_element();
        }

        h_flex()
            .id(SharedString::from(format!(
                "learning-outline-step-{step_index}"
            )))
            .w_full()
            .min_w_0()
            .px_2()
            .py_2()
            .rounded_md()
            .items_center()
            .gap_2()
            .text_sm()
            .when(selected, |item| item.bg(colors.accent))
            .text_color(if selected {
                colors.primary
            } else {
                colors.muted_foreground
            })
            .when(clickable, |item| {
                item.cursor_pointer()
                    .hover(move |style| style.bg(colors.accent.opacity(0.65)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_step(step_index, cx);
                    }))
            })
            .child(icon)
            .child(div().min_w_0().flex_1().truncate().child(label))
            .into_any_element()
    }

    fn render_outline(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let steps = self
            .plan
            .as_ref()
            .map(|plan| plan.steps.as_slice())
            .unwrap_or_default();
        let toggle = Button::new("toggle-learning-outline")
            .small()
            .ghost()
            .icon(if self.outline_collapsed {
                IconName::PanelRightOpen
            } else {
                IconName::PanelRightClose
            })
            .tooltip(if self.outline_collapsed {
                "展开章节进度"
            } else {
                "折叠章节进度"
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_outline(cx)));

        if self.outline_collapsed {
            return v_flex()
                .size_full()
                .child(
                    div()
                        .flex_shrink_0()
                        .w_full()
                        .p_2()
                        .child(h_flex().w_full().justify_center().child(toggle)),
                )
                .child(
                    div()
                        .id("learning-outline-collapsed-scroll-wrap")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(
                            div()
                                .id("learning-outline-collapsed-scroll")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.outline_scroll)
                                .child(
                                    v_flex()
                                        .w_full()
                                        .px_2()
                                        .pb_2()
                                        .items_center()
                                        .gap_2()
                                        .children(
                                            steps.iter().map(|step| {
                                                self.render_outline_item(step, true, cx)
                                            }),
                                        ),
                                ),
                        ),
                )
                .into_any_element();
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .justify_between()
                    .gap_2()
                    .p_4()
                    .child(div().text_sm().font_semibold().child("章节进度"))
                    .child(toggle),
            )
            .child(
                div()
                    .id("learning-outline-scroll-wrap")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .id("learning-outline-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.outline_scroll)
                            .child(
                                v_flex().px_4().pb_4().gap_3().children(
                                    steps
                                        .iter()
                                        .map(|step| self.render_outline_item(step, false, cx)),
                                ),
                            ),
                    )
                    .vertical_scrollbar(&self.outline_scroll),
            )
            .into_any_element()
    }
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<String>().to_lowercase()
}

fn remediation_prompt(prompt: &LearningPrompt) -> LearningPrompt {
    let mut retry = prompt.clone();
    retry.format = QuestionFormat::ShortAnswer;
    retry.options.clear();
    retry.question = if prompt.format == QuestionFormat::Choice {
        format!(
            "请根据下方原文，用自己的话说明原题的正确结论及原因：{}",
            prompt.question
        )
    } else {
        format!(
            "请根据下方原文，用自己的话重新回答原题：{}",
            prompt.question
        )
    };
    retry
}

fn merge_result(current: &mut Option<String>, candidate: &str) {
    fn rank(result: &str) -> usize {
        match result {
            "incorrect" => 0,
            "partial" => 1,
            "correct" => 2,
            _ => 0,
        }
    }
    if current
        .as_deref()
        .is_none_or(|result| rank(candidate) < rank(result))
    {
        *current = Some(candidate.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_result, remediation_prompt};
    use crate::domain::{dynamic_review::QuestionFormat, learning::LearningPrompt};

    #[test]
    fn checkpoint_result_keeps_the_worst_first_attempt() {
        let mut result = None;
        merge_result(&mut result, "correct");
        merge_result(&mut result, "partial");
        merge_result(&mut result, "incorrect");
        merge_result(&mut result, "correct");
        assert_eq!(result.as_deref(), Some("incorrect"));
    }

    #[test]
    fn remediation_question_does_not_reveal_required_points() {
        let prompt = LearningPrompt {
            id: Some(1),
            learning_step_id: 2,
            position: 0,
            unit_ids: vec!["K1".into()],
            format: QuestionFormat::Choice,
            question: "哪一项描述正确？".into(),
            options: vec!["干扰项".into(), "正确项".into()],
            standard_answer: "正确项".into(),
            required_points: vec!["不能出现在题面的秘密判分点".into()],
            source_block_ids: vec!["B1".into()],
            generation_mode: "test".into(),
        };
        let retry = remediation_prompt(&prompt);
        assert_eq!(retry.format, QuestionFormat::ShortAnswer);
        assert!(retry.options.is_empty());
        assert!(!retry.question.contains(&prompt.required_points[0]));
        assert_eq!(retry.standard_answer, prompt.standard_answer);
        assert_eq!(retry.required_points, prompt.required_points);
    }
}
