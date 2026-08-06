use std::collections::HashSet;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::diagnostics;

use super::{
    client::{ChatClient, StreamEvent},
    progress::{ImportCancellation, ImportEvent, ImportEventReporter, ImportProgress, ImportStage},
    prompts::{PLAN_CHUNK_SYSTEM, PLAN_RECONCILE_SYSTEM, PLAN_REPAIR_SYSTEM},
};

const MAX_FINAL_UNITS: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanClaim {
    pub id: String,
    pub statement: String,
    pub importance: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUnit {
    pub id: String,
    pub topic: String,
    pub objective: String,
    pub unit_type: String,
    pub importance: String,
    pub stage: String,
    pub cognitive_action: String,
    pub required_points: Vec<String>,
    #[serde(default)]
    pub claim_ids: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub reason: String,
    #[serde(default)]
    pub quick: bool,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub prerequisite_unit_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialPlan {
    pub summary: String,
    pub document_type: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub claims: Vec<PlanClaim>,
    pub units: Vec<PlanUnit>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChunkPlan {
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    claims: Vec<PlanClaim>,
    units: Vec<PlanUnit>,
}

pub async fn analyze_material(
    client: &ChatClient,
    title: &str,
    content: &str,
) -> Result<MaterialPlan> {
    let silent = |_: ImportEvent| {};
    let cancellation = ImportCancellation::default();
    analyze_material_with_progress(client, title, content, &silent, &cancellation).await
}

pub async fn analyze_material_with_progress(
    client: &ChatClient,
    title: &str,
    content: &str,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<MaterialPlan> {
    cancellation.ensure_active()?;
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Extracting,
        format!(
            "正在一次性分析《{title}》全文（{} 个字符）",
            content.chars().count()
        ),
    )));
    let input = serde_json::json!({
        "material_title": title,
        "content": content,
    });
    let report_stream = |event| {
        if let StreamEvent::Thinking(text) = event {
            progress(ImportEvent::Thinking(text));
        }
    };
    let value = client
        .chat_json_stream_for(
            "plan.extract",
            PLAN_CHUNK_SYSTEM,
            &input.to_string(),
            &report_stream,
            Some(cancellation),
        )
        .await?;
    let mut candidate: ChunkPlan =
        serde_json::from_value(value).map_err(|error| anyhow!("知识提取响应格式不对: {error}"))?;
    prefix_chunk_ids(&mut candidate, 1);
    let candidates = vec![candidate];

    cancellation.ensure_active()?;
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Reconciling,
        format!("正在去重并整理《{title}》的最终知识蓝图"),
    )));
    let plan = reconcile_once(client, title, &candidates, progress, cancellation).await?;
    match validate_plan(plan.clone()) {
        Ok(plan) => Ok(plan),
        Err(initial_error) => {
            diagnostics::warn(
                "plan.validation.repair_started",
                "Knowledge plan failed validation; requesting a targeted schema repair",
                serde_json::json!({
                    "title": title,
                    "error": format!("{initial_error:#}"),
                    "claims": plan.claims.len(),
                    "units": plan.units.len(),
                }),
            );
            progress(ImportEvent::Stage(ImportProgress::stage(
                ImportStage::Reconciling,
                format!("正在修复《{title}》知识蓝图中的结构字段"),
            )));
            let repaired = repair_plan(
                client,
                title,
                &plan,
                &initial_error.to_string(),
                progress,
                cancellation,
            )
            .await
            .map_err(|repair_error| {
                anyhow!("知识蓝图首次校验失败：{initial_error}；AI 结构修复失败：{repair_error}")
            })?;
            validate_plan(repaired).map_err(|repair_error| {
                diagnostics::error(
                    "plan.validation.repair_failed",
                    "Repaired knowledge plan still failed validation",
                    serde_json::json!({
                        "title": title,
                        "initial_error": format!("{initial_error:#}"),
                        "repair_error": format!("{repair_error:#}"),
                    }),
                );
                anyhow!("知识蓝图首次校验失败：{initial_error}；修复后仍未通过：{repair_error}")
            })
        }
    }
}

async fn reconcile_once(
    client: &ChatClient,
    title: &str,
    candidates: &[ChunkPlan],
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<MaterialPlan> {
    let input = serde_json::json!({
        "material_title": title,
        "candidate_chunks": candidates,
    });
    let report_stream = |event| {
        if let StreamEvent::Thinking(text) = event {
            progress(ImportEvent::Thinking(text));
        }
    };
    let value = client
        .chat_json_stream_for(
            "plan.reconcile",
            PLAN_RECONCILE_SYSTEM,
            &input.to_string(),
            &report_stream,
            Some(cancellation),
        )
        .await?;
    serde_json::from_value(value).map_err(|error| anyhow!("知识蓝图响应格式不对: {error}"))
}

async fn repair_plan(
    client: &ChatClient,
    title: &str,
    plan: &MaterialPlan,
    validation_error: &str,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<MaterialPlan> {
    let input = serde_json::json!({
        "material_title": title,
        "validation_error": validation_error,
        "draft_plan": plan,
    });
    let report_stream = |event| {
        if let StreamEvent::Thinking(text) = event {
            progress(ImportEvent::Thinking(text));
        }
    };
    let value = client
        .chat_json_stream_for(
            "plan.repair",
            PLAN_REPAIR_SYSTEM,
            &input.to_string(),
            &report_stream,
            Some(cancellation),
        )
        .await?;
    serde_json::from_value(value).map_err(|error| anyhow!("知识蓝图修复响应格式不对: {error}"))
}

fn prefix_chunk_ids(plan: &mut ChunkPlan, source_index: usize) {
    let prefix = format!("P{source_index}-");
    for claim in &mut plan.claims {
        claim.id = format!("{prefix}{}", claim.id.trim());
    }
    for unit in &mut plan.units {
        unit.id = format!("{prefix}{}", unit.id.trim());
        for claim_id in &mut unit.claim_ids {
            *claim_id = format!("{prefix}{}", claim_id.trim());
        }
        for prerequisite in &mut unit.prerequisite_unit_ids {
            *prerequisite = format!("{prefix}{}", prerequisite.trim());
        }
    }
}

fn validate_plan(mut plan: MaterialPlan) -> Result<MaterialPlan> {
    if plan.units.is_empty() {
        return Err(anyhow!("AI 没有从材料中识别出可复习的知识单元"));
    }
    if plan.units.len() > MAX_FINAL_UNITS {
        return Err(anyhow!(
            "这份材料识别出 {} 个知识单元，超过单篇上限 {MAX_FINAL_UNITS}，建议分批导入",
            plan.units.len()
        ));
    }
    let valid_importance = ["core", "supporting", "detail"];
    let valid_types = [
        "concept",
        "relation",
        "mechanism",
        "procedure",
        "boundary",
        "application",
    ];
    let valid_stages = ["foundation", "relationship", "application"];
    let valid_actions = [
        "recall", "explain", "compare", "sequence", "diagnose", "decide",
    ];
    let claim_ids = plan
        .claims
        .iter()
        .map(|claim| claim.id.clone())
        .collect::<HashSet<_>>();
    if claim_ids.len() != plan.claims.len() {
        return Err(anyhow!("知识蓝图包含重复的 Claim id"));
    }
    for claim in &mut plan.claims {
        claim.statement = claim.statement.trim().chars().take(500).collect();
        claim.evidence.retain(|quote| !quote.trim().is_empty());
        if claim.id.trim().is_empty()
            || claim.statement.is_empty()
            || claim.evidence.is_empty()
            || !valid_importance.contains(&claim.importance.as_str())
        {
            return Err(anyhow!("知识蓝图包含无效的 Claim"));
        }
    }
    let unit_ids = plan
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<HashSet<_>>();
    if unit_ids.len() != plan.units.len() {
        return Err(anyhow!("知识蓝图包含重复的 KnowledgeUnit id"));
    }
    for unit in &mut plan.units {
        unit.topic = unit.topic.trim().chars().take(120).collect();
        unit.objective = unit.objective.trim().chars().take(300).collect();
        unit.reason = unit.reason.trim().chars().take(500).collect();
        unit.required_points
            .retain(|point| !point.trim().is_empty());
        unit.evidence.retain(|quote| !quote.trim().is_empty());
        if unit.id.trim().is_empty()
            || unit.objective.is_empty()
            || unit.required_points.is_empty()
            || unit.evidence.is_empty()
        {
            return Err(anyhow!("知识蓝图包含缺少目标、必答点或证据的单元"));
        }
        if !valid_importance.contains(&unit.importance.as_str())
            || !valid_types.contains(&unit.unit_type.as_str())
            || !valid_stages.contains(&unit.stage.as_str())
            || !valid_actions.contains(&unit.cognitive_action.as_str())
        {
            return Err(anyhow!("知识单元 {} 使用了非法分类", unit.id));
        }
        if unit.quick && !unit.recommended {
            unit.recommended = true;
        }
        if unit.claim_ids.iter().any(|id| !claim_ids.contains(id)) {
            return Err(anyhow!("知识单元 {} 引用了不存在的 Claim", unit.id));
        }
        if unit
            .prerequisite_unit_ids
            .iter()
            .any(|id| id == &unit.id || !unit_ids.contains(id))
        {
            return Err(anyhow!("知识单元 {} 的前置关系无效", unit.id));
        }
    }
    if !plan.units.iter().any(|unit| unit.recommended) {
        return Err(anyhow!("知识蓝图没有推荐任何学习单元"));
    }
    plan.summary = plan.summary.trim().chars().take(600).collect();
    plan.document_type = plan.document_type.trim().chars().take(40).collect();
    plan.warnings = plan
        .warnings
        .into_iter()
        .filter_map(|warning| {
            let warning = warning.trim();
            (!warning.is_empty()).then(|| warning.chars().take(500).collect())
        })
        .collect();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::{MaterialPlan, PlanClaim, PlanUnit, validate_plan};

    fn valid_plan() -> MaterialPlan {
        MaterialPlan {
            summary: "摘要".into(),
            document_type: "concept".into(),
            warnings: vec![],
            claims: vec![PlanClaim {
                id: "C1".into(),
                statement: "事实".into(),
                importance: "core".into(),
                evidence: vec!["原文".into()],
            }],
            units: vec![PlanUnit {
                id: "K1".into(),
                topic: "主题".into(),
                objective: "能够说明事实".into(),
                unit_type: "concept".into(),
                importance: "core".into(),
                stage: "foundation".into(),
                cognitive_action: "recall".into(),
                required_points: vec!["事实".into()],
                claim_ids: vec!["C1".into()],
                evidence: vec!["原文".into()],
                reason: "核心内容".into(),
                quick: true,
                recommended: false,
                prerequisite_unit_ids: vec![],
            }],
        }
    }

    #[test]
    fn quick_unit_is_always_recommended() {
        let plan = validate_plan(valid_plan()).unwrap();
        assert!(plan.units[0].recommended);
    }

    #[test]
    fn rejects_missing_claim_reference() {
        let mut plan = valid_plan();
        plan.units[0].claim_ids = vec!["missing".into()];
        assert!(validate_plan(plan).is_err());
    }
}
