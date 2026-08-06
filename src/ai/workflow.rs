use anyhow::{Result, anyhow};

use crate::diagnostics;

use super::{
    client::ChatClient,
    generate::{Question, generate_questions_with_progress},
    import::{ImportedMaterial, import_materials},
    plan::{MaterialPlan, PlanUnit, analyze_material_with_progress},
    progress::{ImportProgress, ImportProgressReporter},
};

const MAX_QUESTIONS_PER_IMPORT: usize = 160;

#[derive(Debug, Clone)]
pub struct PreparedMaterial {
    pub material: ImportedMaterial,
    pub plan: MaterialPlan,
    pub questions: Vec<Question>,
}

/// 完整的一键导入工作流：去噪拆文档、建立知识蓝图、生成复习基础题。
pub async fn prepare_import_with_progress(
    client: &ChatClient,
    raw: &str,
    progress: &ImportProgressReporter,
) -> Result<Vec<PreparedMaterial>> {
    progress(ImportProgress::preparing());
    diagnostics::info(
        "import.workflow.started",
        "Smart import workflow started",
        serde_json::json!({ "raw_chars": raw.chars().count() }),
    );
    let materials = import_materials(client, raw, progress)
        .await
        .map_err(|error| {
            diagnostics::error(
                "import.workflow.clean_failed",
                "Material cleaning or organization failed",
                serde_json::json!({ "error": format!("{error:#}") }),
            );
            error
        })?;
    diagnostics::info(
        "import.workflow.materials_ready",
        "Clean materials are ready",
        serde_json::json!({
            "material_count": materials.len(),
            "materials": materials.iter().map(|material| serde_json::json!({
                "title": material.title,
                "content_chars": material.content.chars().count(),
                "document_type": material.document_type,
            })).collect::<Vec<_>>(),
        }),
    );
    let mut planned =
        Vec::<(ImportedMaterial, MaterialPlan, Vec<PlanUnit>)>::with_capacity(materials.len());
    let mut total_questions = 0;
    for (index, material) in materials.into_iter().enumerate() {
        diagnostics::info(
            "import.workflow.plan_started",
            "Knowledge planning started",
            serde_json::json!({
                "material_index": index + 1,
                "title": material.title,
                "content_chars": material.content.chars().count(),
            }),
        );
        let mut plan =
            analyze_material_with_progress(client, &material.title, &material.content, progress)
                .await
                .map_err(|error| {
                    diagnostics::error(
                        "import.workflow.plan_failed",
                        "Knowledge planning failed",
                        serde_json::json!({
                            "material_index": index + 1,
                            "title": material.title,
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
        total_questions += selected.len();
        if total_questions > MAX_QUESTIONS_PER_IMPORT {
            return Err(anyhow!(
                "本次材料建议生成超过 {MAX_QUESTIONS_PER_IMPORT} 道题，请拆成两次导入"
            ));
        }
        diagnostics::info(
            "import.workflow.plan_ready",
            "Knowledge plan is ready",
            serde_json::json!({
                "material_index": index + 1,
                "title": material.title,
                "claims": plan.claims.len(),
                "units": plan.units.len(),
                "recommended_units": selected.len(),
                "warnings": plan.warnings,
            }),
        );
        planned.push((material, plan, selected));
    }

    let mut prepared = Vec::with_capacity(planned.len());
    for (index, (material, plan, selected)) in planned.into_iter().enumerate() {
        let questions =
            generate_questions_with_progress(client, &selected, &material.title, progress)
                .await
                .map_err(|error| {
                    diagnostics::error(
                        "import.workflow.questions_failed",
                        "Question generation failed",
                        serde_json::json!({
                            "material_index": index + 1,
                            "title": material.title,
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
            "question_count": total_questions,
        }),
    );
    Ok(prepared)
}
