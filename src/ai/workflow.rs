use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Result, anyhow};

use crate::diagnostics;

use super::{
    client::ChatClient,
    generate::{Question, generate_questions_with_progress},
    image::{VisionClient, append_image_context, describe_images},
    import::{ImportedMaterial, import_materials},
    plan::{MaterialPlan, PlanUnit, analyze_material_with_progress},
    progress::{ImportCancellation, ImportEvent, ImportEventReporter, ImportProgress, ImportStage},
    source::SourceBundle,
};

const MAX_QUESTIONS_PER_IMPORT: usize = 160;
const MAX_STAGE_ATTEMPTS: usize = 2;
static NEXT_WORKFLOW_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct PreparedMaterial {
    pub material: ImportedMaterial,
    pub plan: MaterialPlan,
    pub questions: Vec<Question>,
}

/// Prepares a scanned local source. Images are read and uploaded only when an
/// independent vision client is configured; otherwise they are ignored.
pub async fn prepare_source_bundle_with_progress(
    client: &ChatClient,
    vision: Option<&VisionClient>,
    mut source: SourceBundle,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<Vec<PreparedMaterial>> {
    diagnostics::info(
        "import.source.scanned",
        "Local source scan completed",
        serde_json::json!({
            "root": source.root.display().to_string(),
            "text_files": source.files.len(),
            "images": source.images.len(),
            "text_chars": source.text.chars().count(),
            "warnings": &source.warnings,
            "vision_enabled": vision.is_some(),
        }),
    );
    if let Some(vision) = vision
        && !source.images.is_empty()
    {
        let described = describe_images(vision, &source.images, progress, cancellation).await?;
        append_image_context(&mut source.text, &described);
    }
    if source.text.trim().is_empty() {
        return Err(anyhow!(
            "扫描结果没有可导入文本；图像识别未配置时会忽略目录中的图片"
        ));
    }
    prepare_import_with_progress(client, &source.text, progress, cancellation).await
}

/// 完整的一键导入工作流：去噪拆文档、建立知识蓝图、生成复习基础题。
pub async fn prepare_import_with_progress(
    client: &ChatClient,
    raw: &str,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<Vec<PreparedMaterial>> {
    let workflow_id = NEXT_WORKFLOW_ID.fetch_add(1, Ordering::Relaxed);
    cancellation.ensure_active()?;
    progress(ImportEvent::Stage(ImportProgress::preparing()));
    diagnostics::info(
        "import.workflow.started",
        "Smart import workflow started",
        serde_json::json!({
            "workflow_id": workflow_id,
            "raw_chars": raw.chars().count(),
        }),
    );
    let materials = run_stage_with_recovery(
        "import.clean_and_organize",
        ImportStage::Cleaning,
        progress,
        cancellation,
        workflow_id,
        || import_materials(client, raw, progress, cancellation),
    )
    .await
    .map_err(|error| {
        diagnostics::error(
            "import.workflow.clean_failed",
            "Material cleaning or organization failed",
            serde_json::json!({
                "workflow_id": workflow_id,
                "error": format!("{error:#}"),
            }),
        );
        error
    })?;
    diagnostics::info(
        "import.workflow.materials_ready",
        "Clean materials are ready",
        serde_json::json!({
            "material_count": materials.len(),
            "workflow_id": workflow_id,
            "materials": materials.iter().map(|material| serde_json::json!({
                "content_chars": material.content.chars().count(),
                "document_type": material.document_type,
            })).collect::<Vec<_>>(),
        }),
    );
    let mut planned =
        Vec::<(ImportedMaterial, MaterialPlan, Vec<PlanUnit>)>::with_capacity(materials.len());
    for (index, material) in materials.into_iter().enumerate() {
        diagnostics::info(
            "import.workflow.plan_started",
            "Knowledge planning started",
            serde_json::json!({
                "material_index": index + 1,
                "workflow_id": workflow_id,
                "content_chars": material.content.chars().count(),
            }),
        );
        let mut plan = run_stage_with_recovery(
            "plan.analyze_and_validate",
            ImportStage::Extracting,
            progress,
            cancellation,
            workflow_id,
            || {
                analyze_material_with_progress(
                    client,
                    &material.title,
                    &material.content,
                    progress,
                    cancellation,
                )
            },
        )
        .await
        .map_err(|error| {
            diagnostics::error(
                "import.workflow.plan_failed",
                "Knowledge planning failed",
                serde_json::json!({
                    "material_index": index + 1,
                    "workflow_id": workflow_id,
                    "error": format!("{error:#}"),
                }),
            );
            error
        })?;
        if plan.summary.trim().is_empty() {
            plan.summary = material.summary.clone();
        }
        if plan.document_type.trim().is_empty() {
            plan.document_type = material.document_type.clone();
        }
        let selected = plan
            .units
            .iter()
            .filter(|unit| unit.recommended)
            .cloned()
            .collect::<Vec<_>>();
        diagnostics::info(
            "import.workflow.plan_ready",
            "Knowledge plan is ready",
            serde_json::json!({
                "material_index": index + 1,
                "workflow_id": workflow_id,
                "claims": plan.claims.len(),
                "units": plan.units.len(),
                "recommended_units": selected.len(),
                "warning_count": plan.warnings.len(),
            }),
        );
        planned.push((material, plan, selected));
    }

    let selected_before_cap = planned
        .iter()
        .map(|(_, _, units)| units.len())
        .sum::<usize>();
    let total_questions = cap_question_units(&mut planned, MAX_QUESTIONS_PER_IMPORT);
    if selected_before_cap > total_questions {
        diagnostics::warn(
            "import.workflow.questions_capped",
            "Seed question generation was capped for a large import",
            serde_json::json!({
                "workflow_id": workflow_id,
                "selected_units": selected_before_cap,
                "generated_seed_questions": total_questions,
                "cap": MAX_QUESTIONS_PER_IMPORT,
            }),
        );
    }

    cancellation.ensure_active()?;
    let mut prepared = Vec::with_capacity(planned.len());
    for (index, (material, plan, selected)) in planned.into_iter().enumerate() {
        cancellation.ensure_active()?;
        let questions = run_stage_with_recovery(
            "questions.generate",
            ImportStage::Generating,
            progress,
            cancellation,
            workflow_id,
            || {
                generate_questions_with_progress(
                    client,
                    &selected,
                    &material.title,
                    progress,
                    cancellation,
                )
            },
        )
        .await
        .map_err(|error| {
            diagnostics::error(
                "import.workflow.questions_failed",
                "Question generation failed",
                serde_json::json!({
                    "material_index": index + 1,
                    "workflow_id": workflow_id,
                    "unit_count": selected.len(),
                    "error": format!("{error:#}"),
                }),
            );
            error
        })?;
        prepared.push(PreparedMaterial {
            material,
            plan,
            questions,
        });
    }
    diagnostics::info(
        "import.workflow.completed",
        "Smart import workflow completed",
        serde_json::json!({
            "material_count": prepared.len(),
            "workflow_id": workflow_id,
            "question_count": total_questions,
        }),
    );
    Ok(prepared)
}

fn cap_question_units(
    planned: &mut [(ImportedMaterial, MaterialPlan, Vec<PlanUnit>)],
    max: usize,
) -> usize {
    let total = planned
        .iter()
        .map(|(_, _, units)| units.len())
        .sum::<usize>();
    if total <= max {
        return total;
    }
    for (_, _, units) in planned.iter_mut() {
        units.sort_by_key(|unit| {
            (
                usize::from(!unit.quick),
                match unit.importance.as_str() {
                    "core" => 0,
                    "supporting" => 1,
                    _ => 2,
                },
                match unit.stage.as_str() {
                    "foundation" => 0,
                    "relationship" => 1,
                    _ => 2,
                },
            )
        });
    }
    let mut retained = vec![Vec::new(); planned.len()];
    let mut positions = vec![0usize; planned.len()];
    let mut count = 0usize;
    while count < max {
        let mut advanced = false;
        for (index, (_, _, units)) in planned.iter().enumerate() {
            if count == max {
                break;
            }
            if let Some(unit) = units.get(positions[index]) {
                retained[index].push(unit.clone());
                positions[index] += 1;
                count += 1;
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    for ((_, _, units), retained) in planned.iter_mut().zip(retained) {
        *units = retained;
    }
    count
}

/// Runs one workflow stage with a bounded recovery attempt.  The retry is
/// deliberately at the stage boundary: all downstream data is still derived
/// from validated output, so a failed contract never leaks partial state.
async fn run_stage_with_recovery<T, F, Fut>(
    operation: &'static str,
    stage: ImportStage,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
    workflow_id: u64,
    mut action: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last_error = None;
    for attempt in 1..=MAX_STAGE_ATTEMPTS {
        cancellation.ensure_active()?;
        match action().await {
            Ok(value) => {
                if attempt > 1 {
                    diagnostics::info(
                        "ai.workflow.stage_recovered",
                        "AI workflow stage recovered after retry",
                        serde_json::json!({
                            "operation": operation,
                            "workflow_id": workflow_id,
                            "attempt": attempt,
                        }),
                    );
                }
                return Ok(value);
            }
            Err(error) if attempt < MAX_STAGE_ATTEMPTS => {
                diagnostics::warn(
                    "ai.workflow.stage_retrying",
                    "AI workflow stage failed; retrying from a clean stage boundary",
                    serde_json::json!({
                        "operation": operation,
                        "workflow_id": workflow_id,
                        "attempt": attempt,
                        "next_attempt": attempt + 1,
                        "error": format!("{error:#}"),
                    }),
                );
                progress(ImportEvent::Stage(ImportProgress::stage(
                    stage,
                    format!(
                        "本阶段第 {attempt} 次尝试未通过校验，正在重新执行（最多 {MAX_STAGE_ATTEMPTS} 次）"
                    ),
                )));
                last_error = Some(error);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("stage runner must record an error"))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::run_stage_with_recovery;
    use crate::ai::progress::{ImportCancellation, ImportEvent, ImportStage};

    #[tokio::test]
    async fn retries_a_failed_stage_once_and_reports_recovery() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(AtomicUsize::new(0));
        let cancellation = ImportCancellation::default();
        let report = {
            let events = events.clone();
            move |event| {
                if matches!(event, ImportEvent::Stage(_)) {
                    events.fetch_add(1, Ordering::Relaxed);
                }
            }
        };
        let result = run_stage_with_recovery(
            "test.stage",
            ImportStage::Cleaning,
            &report,
            &cancellation,
            1,
            {
                let attempts = attempts.clone();
                move || {
                    let attempt = attempts.fetch_add(1, Ordering::Relaxed);
                    async move {
                        if attempt == 0 {
                            anyhow::bail!("contract")
                        } else {
                            Ok(42)
                        }
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert_eq!(events.load(Ordering::Relaxed), 1);
    }
}
