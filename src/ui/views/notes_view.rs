//! 笔记视图：导入学习材料、AI 出题、浏览卡片。

use gpui::{
    AnyWindowHandle, Context, Entity, IntoElement, Render, ScrollHandle, SharedString, Window, div,
    point, prelude::*, px,
};
use gpui_component::Disableable as _;
use gpui_component::alert::Alert;
use gpui_component::breadcrumb::{Breadcrumb, BreadcrumbItem};
use gpui_component::button::{Button, ButtonGroup, ButtonVariant, ButtonVariants as _};
use gpui_component::dialog::DialogButtonProps;
use gpui_component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::skeleton::Skeleton;
use gpui_component::spinner::Spinner;
use gpui_component::theme::ActiveTheme as _;
use gpui_component::{
    Icon, IconName, Placement, Root, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _,
    h_flex, v_flex,
};

use crate::ai::progress::{ImportProgress, ImportStage};
use crate::assets::RuizIcon;
use crate::db;
use crate::domain::card::Card;
use crate::domain::knowledge::{KnowledgeUnit, MaterialAnalysis};
use crate::domain::note::Note;
use crate::state::AppState;
use crate::ui::components::{empty_state, page_header};

pub struct NotesView {
    notes: Vec<Note>,
    /// 笔记页内部导航状态，实体在切换主 Tab 时不会重建。
    page: NotesPage,
    cards: Vec<Card>,
    content: Entity<InputState>,
    analysis: Option<MaterialAnalysis>,
    units: Vec<KnowledgeUnit>,
    notes_loading: bool,
    cards_loading: bool,
    importing: bool,
    import_progress: Option<ImportProgress>,
    generating: bool,
    generation_scope: GenerationScope,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GenerationScope {
    Quick,
    #[default]
    Recommended,
    Comprehensive,
}

impl GenerationScope {
    fn includes(self, quick: bool, recommended: bool) -> bool {
        match self {
            Self::Quick => quick,
            Self::Recommended => recommended,
            Self::Comprehensive => true,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Quick => "精简",
            Self::Recommended => "AI 建议",
            Self::Comprehensive => "全面",
        }
    }

    fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Quick),
            1 => Some(Self::Recommended),
            2 => Some(Self::Comprehensive),
            _ => None,
        }
    }
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
        let content = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("把一篇或多篇网页、Markdown、课程笔记直接粘贴到这里…")
                .multi_line(true)
                .rows(16)
        });
        let mut view = Self {
            notes: Vec::new(),
            page: NotesPage::Library,
            cards: Vec::new(),
            content,
            analysis: None,
            units: Vec::new(),
            notes_loading: true,
            cards_loading: false,
            importing: false,
            import_progress: None,
            generating: false,
            generation_scope: GenerationScope::default(),
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
                                    this.analysis = None;
                                    this.units.clear();
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
                        let cards = db::cards::by_note(&pool, note_id).await?;
                        let analysis = db::knowledge::analysis_by_note(&pool, note_id).await?;
                        let units = db::knowledge::units_by_note(&pool, note_id).await?;
                        anyhow::Ok((cards, analysis, units))
                    })
                    .await;
                    match result {
                        Ok((cards, analysis, units)) => {
                            this.update(&mut cx, |this, cx| {
                                if this.page.note_id() == Some(note_id) {
                                    this.cards = cards;
                                    this.analysis = analysis;
                                    this.units = units;
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

    fn import_materials(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let pool = AppState::global(cx).pool.clone();
        let content = self.content.read(cx).value().to_string();
        if content.trim().is_empty() {
            window.push_notification("请先粘贴学习材料", cx);
            cx.notify();
            return false;
        }
        let Some(ai) = AppState::global(cx).ai.clone() else {
            window.push_notification("请先在设置中配置 AI", cx);
            return false;
        };
        self.importing = true;
        self.import_progress = Some(ImportProgress::preparing());
        self.message = None;
        cx.notify();
        let content_input = self.content.clone();
        let window_handle = self.window;
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    while let Some(progress) = progress_rx.recv().await {
                        this.update(&mut cx, |this, cx| {
                            if this.importing {
                                this.import_progress = Some(progress);
                                cx.notify();
                            }
                        })
                        .ok();
                    }
                }
            },
        )
        .detach();
        cx.spawn(
            move |this: gpui::WeakEntity<NotesView>, cx: &mut gpui::AsyncApp| {
                let this = this.clone();
                let mut cx = (*cx).clone();
                async move {
                    let result = gpui_tokio::Tokio::spawn_result(&cx, async move {
                        let report = move |progress| {
                            let _ = progress_tx.send(progress);
                        };
                        let prepared = crate::ai::workflow::prepare_import_with_progress(
                            &ai, &content, &report,
                        )
                        .await?;
                        report(ImportProgress::stage(
                            ImportStage::Saving,
                            "AI 结果已经校验完成，正在原子写入资料库",
                        ));
                        db::knowledge::save_import(&pool, &prepared).await
                    })
                    .await;
                    match result {
                        Ok(summary) => {
                            let material_count = summary.note_ids.len();
                            let question_count = summary.question_count;
                            let only_note = (material_count == 1).then(|| summary.note_ids[0]);
                            crate::diagnostics::info(
                                "import.persistence.completed",
                                "Smart import was saved to the database",
                                serde_json::json!({
                                    "material_count": material_count,
                                    "question_count": question_count,
                                    "note_ids": summary.note_ids,
                                }),
                            );
                            cx.update_window(window_handle, move |_view, window, cx| {
                                content_input.update(cx, |s, cx| s.set_value("", window, cx));
                                window.push_notification(
                                    format!(
                                        "已导入 {material_count} 篇材料，生成 {question_count} 张推荐卡片"
                                    ),
                                    cx,
                                );
                            })
                            .ok();
                            this.update(&mut cx, |this, cx| {
                                this.importing = false;
                                this.import_progress = None;
                                this.page = only_note
                                    .map(NotesPage::Detail)
                                    .unwrap_or(NotesPage::Library);
                                this.message = Some(Message::Success(format!(
                                    "AI 已整理出 {material_count} 篇材料，并生成 {question_count} 张推荐卡片"
                                )));
                                this.refresh_notes(cx);
                                if only_note.is_some() {
                                    this.refresh_cards(cx);
                                }
                            })
                            .ok();
                        }
                        Err(e) => {
                            crate::diagnostics::error(
                                "import.ui.failed",
                                "Smart import failed before persistence completed",
                                serde_json::json!({ "error": format!("{e:#}") }),
                            );
                            this.update(&mut cx, |this, cx| {
                                this.importing = false;
                                this.import_progress = None;
                                this.message = Some(Message::Error(format!(
                                    "智能导入失败: {e}\n{}",
                                    crate::diagnostics::log_hint()
                                )));
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
        self.analysis = None;
        self.units.clear();
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        self.refresh_cards(cx);
        cx.notify();
    }

    fn show_library(&mut self, cx: &mut Context<Self>) {
        self.page = NotesPage::Library;
        self.cards_loading = false;
        self.analysis = None;
        self.units.clear();
        cx.notify();
    }

    fn open_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match crate::diagnostics::open_log_directory() {
            Ok(()) => window.push_notification("已打开诊断日志目录", cx),
            Err(error) => {
                crate::diagnostics::error(
                    "diagnostics.open_failed",
                    "Failed to open diagnostics directory",
                    serde_json::json!({ "error": format!("{error:#}") }),
                );
                window.push_notification(format!("打开日志目录失败: {error}"), cx);
            }
        }
    }

    fn open_import_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.content.clone();
        let view = cx.entity();
        let sheet_width = px((window.viewport_size().width.as_f32() - 24.).clamp(340., 680.));
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
                                    "直接粘贴原始网页、课程笔记或多篇 Markdown。AI 会自动去除导航与广告，拆分材料，生成标题、知识蓝图和推荐卡片。",
                                ),
                        )
                        .child(
                            v_flex()
                                .min_h_0()
                                .flex_1()
                                .gap_1()
                                .child(div().text_sm().font_medium().child("原始材料"))
                                .child(Input::new(&content).h(px(440.)))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("单次最多 300,000 个字符，可以包含网页菜单、目录和多篇文章。"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::diagnostics::log_hint()),
                                ),
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
                            Button::new("smart-import")
                                .icon(RuizIcon::Sparkles)
                                .label("AI 整理并导入")
                                .primary()
                                .on_click(move |_, window, cx| {
                                    let started = save_view
                                        .update(cx, |this, cx| this.import_materials(window, cx));
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
                                    this.analysis = None;
                                    this.units.clear();
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

    /// 为旧笔记补建蓝图，或按当前范围生成尚未生成的知识单元。
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
                "请先在「设置」里配置 DeepSeek API 密钥".into(),
            ));
            cx.notify();
            return;
        };
        let has_analysis = self.analysis.is_some();
        let generation_scope = self.generation_scope;
        let remaining_units = self
            .units
            .iter()
            .filter(|unit| {
                !unit.generated && generation_scope.includes(unit.quick, unit.recommended)
            })
            .cloned()
            .collect::<Vec<_>>();
        if has_analysis && remaining_units.is_empty() {
            self.message = Some(Message::Success(format!(
                "{}范围的卡片已经全部生成",
                generation_scope.label()
            )));
            cx.notify();
            return;
        }
        let title = note.title.clone();
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
                        if has_analysis {
                            let units = remaining_units
                                .iter()
                                .map(crate::ai::plan::PlanUnit::from)
                                .collect::<Vec<_>>();
                            let questions =
                                crate::ai::generate::generate_questions(&ai, &units).await?;
                            db::knowledge::save_generated_questions(&pool, note_id, &questions)
                                .await?;
                            anyhow::Ok(questions.len())
                        } else {
                            let plan =
                                crate::ai::plan::analyze_material(&ai, &title, &content).await?;
                            let selected = plan
                                .units
                                .iter()
                                .filter(|unit| {
                                    generation_scope.includes(unit.quick, unit.recommended)
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            let questions =
                                crate::ai::generate::generate_questions(&ai, &selected).await?;
                            let prepared = crate::ai::workflow::PreparedMaterial {
                                material: crate::ai::import::ImportedMaterial {
                                    title,
                                    content: content.clone(),
                                    raw_content: content,
                                    summary: String::new(),
                                    document_type: "mixed".into(),
                                },
                                plan,
                                questions,
                            };
                            db::knowledge::save_plan_for_note(&pool, note_id, &prepared).await
                        }
                    })
                    .await;
                    match result {
                        Ok(n) => {
                            this.update(&mut cx, |this, cx| {
                                this.generating = false;
                                this.message = Some(Message::Success(format!(
                                    "知识蓝图已更新，并生成 {n} 道题"
                                )));
                                this.refresh_cards(cx);
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            crate::diagnostics::error(
                                "questions.ui.failed",
                                "Knowledge plan or question generation failed",
                                serde_json::json!({ "error": format!("{e:#}") }),
                            );
                            this.update(&mut cx, |this, cx| {
                                this.generating = false;
                                this.message = Some(Message::Error(format!(
                                    "出题失败: {e}\n{}",
                                    crate::diagnostics::log_hint()
                                )));
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
        let import_progress = self
            .import_progress
            .clone()
            .unwrap_or_else(ImportProgress::preparing);
        let import_stage_position = import_progress.stage.position();
        let import_stage_counter = if import_stage_position == 0 {
            "正在启动工作流".to_string()
        } else {
            format!("阶段 {import_stage_position}/{}", ImportStage::TOTAL)
        };

        let header = match self.page {
            NotesPage::Library => page_header(
                RuizIcon::NotebookText,
                "资料库",
                "集中管理学习材料，打开一篇笔记后再生成和浏览学习卡片。",
                Some(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("open-diagnostics")
                                .icon(IconName::File)
                                .label("日志")
                                .outline()
                                .tooltip("打开诊断日志目录")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_logs(window, cx);
                                })),
                        )
                        .child(
                            Button::new("btn-import")
                                .icon(IconName::Plus)
                                .label(if self.importing {
                                    "AI 整理中…"
                                } else {
                                    "导入材料"
                                })
                                .primary()
                                .loading(self.importing)
                                .disabled(self.importing)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_import_sheet(window, cx);
                                })),
                        )
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
                    "查看整理后的正文、知识蓝图和 AI 推荐卡片。",
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
                    .when(self.importing, |this| {
                        this.child(
                            GroupBox::new()
                                .outline()
                                .w_full()
                                .max_w(px(992.))
                                .mx_auto()
                                .mb_4()
                                .title(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(Spinner::new().small().color(colors.primary))
                                        .child(format!("AI 正在{}", import_progress.stage.label())),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.muted_foreground)
                                        .child(import_progress.detail.clone()),
                                )
                                .child(h_flex().w_full().gap_1().children(
                                    (1..=ImportStage::TOTAL).map(|stage| {
                                        div().h(px(4.)).flex_1().rounded_full().bg(
                                            if stage <= import_stage_position {
                                                colors.primary
                                            } else {
                                                colors.muted
                                            },
                                        )
                                    }),
                                ))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(import_stage_counter.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted_foreground)
                                        .child(crate::diagnostics::log_hint()),
                                )
                                .child(Skeleton::new().secondary().w_4_5()),
                        )
                    })
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
                                        preview(&note.content, if compact { 72 } else { 120 });
                                    div()
                                        .id(SharedString::from(format!("library-note-{id}")))
                                        .w_full()
                                        .max_w(px(992.))
                                        .mx_auto()
                                        .mb_2()
                                        .px_3()
                                        .py_2()
                                        .rounded_lg()
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
                                                .gap_3()
                                                .child(
                                                    h_flex()
                                                        .size_8()
                                                        .flex_shrink_0()
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded_lg()
                                                        .bg(colors.primary.opacity(0.1))
                                                        .text_color(colors.primary)
                                                        .child(
                                                            Icon::new(RuizIcon::NotebookText)
                                                                .size_4(),
                                                        ),
                                                )
                                                .child(
                                                    v_flex()
                                                        .min_w_0()
                                                        .flex_1()
                                                        .gap_0p5()
                                                        .child(
                                                            h_flex()
                                                                .min_w_0()
                                                                .w_full()
                                                                .justify_between()
                                                                .gap_3()
                                                                .child(
                                                                    div()
                                                                        .min_w_0()
                                                                        .flex_1()
                                                                        .truncate()
                                                                        .font_semibold()
                                                                        .child(SharedString::from(
                                                                            title,
                                                                        )),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .flex_shrink_0()
                                                                        .text_xs()
                                                                        .text_color(
                                                                            colors.muted_foreground,
                                                                        )
                                                                        .child(created_at),
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .line_clamp(1)
                                                                .text_xs()
                                                                .text_color(colors.muted_foreground)
                                                                .child(excerpt),
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
            let remaining_count = self
                .units
                .iter()
                .filter(|unit| {
                    !unit.generated && self.generation_scope.includes(unit.quick, unit.recommended)
                })
                .count();
            let quick_label = self.analysis.as_ref().map_or_else(
                || "精简".to_string(),
                |analysis| format!("精简 · {}", analysis.quick_count),
            );
            let recommended_label = self.analysis.as_ref().map_or_else(
                || "AI 建议".to_string(),
                |analysis| format!("AI 建议 · {}", analysis.recommended_count),
            );
            let comprehensive_label = self.analysis.as_ref().map_or_else(
                || "全面".to_string(),
                |analysis| format!("全面 · {}", analysis.comprehensive_count),
            );
            let generate_label = if self.analysis.is_some() {
                if self.generating {
                    "生成中…".to_string()
                } else if remaining_count == 0 {
                    format!("{}已生成", self.generation_scope.label())
                } else {
                    format!("生成{} {remaining_count} 张", self.generation_scope.label())
                }
            } else if self.generating {
                "分析中…".to_string()
            } else {
                format!("建立蓝图并生成{}", self.generation_scope.label())
            };
            let generate_bar = GroupBox::new()
                .outline()
                .title(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(RuizIcon::Sparkles).size_4())
                        .child("AI 学习蓝图"),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .flex_wrap()
                                .child(div().text_sm().font_medium().child("生成范围"))
                                .child(
                                    ButtonGroup::new("generation-scope")
                                        .small()
                                        .outline()
                                        .disabled(self.generating || self.cards_loading)
                                        .children([
                                            Button::new("scope-quick").label(quick_label).selected(
                                                self.generation_scope == GenerationScope::Quick,
                                            ),
                                            Button::new("scope-recommended")
                                                .label(recommended_label)
                                                .selected(
                                                    self.generation_scope
                                                        == GenerationScope::Recommended,
                                                ),
                                            Button::new("scope-comprehensive")
                                                .label(comprehensive_label)
                                                .selected(
                                                    self.generation_scope
                                                        == GenerationScope::Comprehensive,
                                                ),
                                        ])
                                        .on_click(cx.listener(
                                            |this, selected: &Vec<usize>, _, cx| {
                                                if let Some(scope) =
                                                    selected.first().and_then(|index| {
                                                        GenerationScope::from_index(*index)
                                                    })
                                                {
                                                    this.generation_scope = scope;
                                                    cx.notify();
                                                }
                                            },
                                        )),
                                ),
                        )
                        .when(self.analysis.is_none(), |this| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(colors.muted_foreground)
                                    .child("导入旧笔记后，可以建立知识蓝图并生成学习卡片。"),
                            )
                        })
                        .when_some(self.analysis.as_ref(), |this, analysis| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(colors.muted_foreground)
                                    .child(SharedString::from(analysis.summary.clone())),
                            )
                        })
                        .when(
                            self.analysis
                                .as_ref()
                                .is_some_and(|analysis| !analysis.warnings.is_empty()),
                            |this| {
                                let warnings = self
                                    .analysis
                                    .as_ref()
                                    .map(|analysis| analysis.warnings.join("\n"))
                                    .unwrap_or_default();
                                this.child(Alert::warning("analysis-warning", warnings))
                            },
                        )
                        .child(
                            Button::new("btn-generate")
                                .icon(RuizIcon::Sparkles)
                                .label(generate_label)
                                .primary()
                                .loading(self.generating)
                                .disabled(
                                    self.generating
                                        || self.cards_loading
                                        || (self.analysis.is_some() && remaining_count == 0),
                                )
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

            let blueprint = if self.units.is_empty() {
                div().into_any_element()
            } else {
                GroupBox::new()
                    .fill()
                    .title(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::File).size_4())
                            .child(format!("知识蓝图 · {} 个学习目标", self.units.len())),
                    )
                    .child(v_flex().gap_2().children(self.units.iter().map(|unit| {
                        let (importance_label, importance_color) =
                            importance_style(&unit.importance, cx);
                        h_flex()
                            .items_start()
                            .gap_3()
                            .p_3()
                            .rounded_lg()
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.background)
                            .child(div().mt_1().size_2().flex_shrink_0().rounded_full().bg(
                                if unit.generated {
                                    cx.theme().green
                                } else {
                                    colors.muted_foreground
                                },
                            ))
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
                                            .child(
                                                div().text_sm().font_medium().child(
                                                    SharedString::from(unit.objective.clone()),
                                                ),
                                            )
                                            .child(
                                                div()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded_full()
                                                    .bg(importance_color.opacity(0.12))
                                                    .text_xs()
                                                    .text_color(importance_color)
                                                    .child(importance_label),
                                            )
                                            .child(
                                                div()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded_full()
                                                    .bg(colors.muted)
                                                    .text_xs()
                                                    .text_color(colors.muted_foreground)
                                                    .child(unit_type_label(&unit.unit_type)),
                                            ),
                                    )
                                    .child(
                                        div().text_xs().text_color(colors.muted_foreground).child(
                                            format!(
                                                "{} · {} 个必答点 · {}",
                                                unit.topic,
                                                unit.required_points.len(),
                                                if unit.generated {
                                                    "已生成卡片"
                                                } else {
                                                    "等待生成"
                                                }
                                            ),
                                        ),
                                    ),
                            )
                    })))
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
                .child(blueprint)
                .child(
                    GroupBox::new()
                        .fill()
                        .title(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::File).size_4())
                                .child("整理后正文"),
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

fn importance_style(importance: &str, cx: &gpui::App) -> (&'static str, gpui::Hsla) {
    match importance {
        "core" => ("核心", cx.theme().red),
        "supporting" => ("支撑", cx.theme().yellow),
        _ => ("细节", cx.theme().blue),
    }
}

fn unit_type_label(unit_type: &str) -> &'static str {
    match unit_type {
        "concept" => "概念",
        "relation" => "关系",
        "mechanism" => "机制",
        "procedure" => "流程",
        "boundary" => "边界",
        "application" => "应用",
        _ => "知识",
    }
}

#[cfg(test)]
mod tests {
    use super::GenerationScope;

    #[test]
    fn generation_scopes_are_cumulative() {
        assert!(GenerationScope::Quick.includes(true, true));
        assert!(!GenerationScope::Quick.includes(false, true));
        assert!(GenerationScope::Recommended.includes(true, true));
        assert!(GenerationScope::Recommended.includes(false, true));
        assert!(!GenerationScope::Recommended.includes(false, false));
        assert!(GenerationScope::Comprehensive.includes(false, false));
    }

    #[test]
    fn generation_scope_matches_segment_index() {
        assert_eq!(GenerationScope::from_index(0), Some(GenerationScope::Quick));
        assert_eq!(
            GenerationScope::from_index(1),
            Some(GenerationScope::Recommended)
        );
        assert_eq!(
            GenerationScope::from_index(2),
            Some(GenerationScope::Comprehensive)
        );
        assert_eq!(GenerationScope::from_index(3), None);
    }
}
