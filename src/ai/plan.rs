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
const MAX_PLAN_CHUNK_CHARS: usize = 60_000;
const MAX_EVIDENCE_CHARS: usize = 2_000;
const MAX_RECONCILE_INPUT_CHARS: usize = 450_000;
const MAX_RECONCILE_ROUNDS: usize = 12;
const MAX_REPAIR_CONTEXT_CHARS: usize = 400_000;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let chunks = split_plan_chunks(content);
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Extracting,
        format!(
            "正在分段分析《{title}》全文（{} 个字符，共 {} 段）",
            content.chars().count(),
            chunks.len()
        ),
    )));
    let mut candidates = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        cancellation.ensure_active()?;
        let input = serde_json::json!({
            "material_title": title,
            "chunk_index": index + 1,
            "chunk_count": chunks.len(),
            "content": chunk,
        });
        let report_stream = |event| match event {
            StreamEvent::Thinking(text) => progress(ImportEvent::Thinking(text)),
            StreamEvent::Content(text) => progress(ImportEvent::Answer(text)),
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
        let mut candidate: ChunkPlan = serde_json::from_value(value)
            .map_err(|error| anyhow!("知识提取响应格式不对（第 {} 段）: {error}", index + 1))?;
        prefix_chunk_ids(&mut candidate, index + 1);
        candidates.push(candidate);
    }

    cancellation.ensure_active()?;
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Reconciling,
        format!("正在去重并整理《{title}》的最终知识蓝图"),
    )));
    let plan = reconcile_hierarchically(client, title, candidates, progress, cancellation).await?;
    match validate_plan_against_content(plan.clone(), content) {
        Ok(plan) => Ok(plan),
        Err(initial_error) => {
            diagnostics::warn(
                "plan.validation.repair_started",
                "Knowledge plan failed validation; requesting a targeted schema repair",
                serde_json::json!({
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
                content,
                progress,
                cancellation,
            )
            .await
            .map_err(|repair_error| {
                anyhow!("知识蓝图首次校验失败：{initial_error}；AI 结构修复失败：{repair_error}")
            })?;
            validate_plan_against_content(repaired, content).map_err(|repair_error| {
                diagnostics::error(
                    "plan.validation.repair_failed",
                    "Repaired knowledge plan still failed validation",
                    serde_json::json!({
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
    let report_stream = |event| match event {
        StreamEvent::Thinking(text) => progress(ImportEvent::Thinking(text)),
        StreamEvent::Content(text) => progress(ImportEvent::Answer(text)),
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

async fn reconcile_hierarchically(
    client: &ChatClient,
    title: &str,
    mut candidates: Vec<ChunkPlan>,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<MaterialPlan> {
    for round in 1..=MAX_RECONCILE_ROUNDS {
        cancellation.ensure_active()?;
        let batches = reconcile_batches(&candidates);
        if batches.len() == 1 {
            return reconcile_once(client, title, &batches[0], progress, cancellation).await;
        }
        progress(ImportEvent::Stage(ImportProgress::stage(
            ImportStage::Reconciling,
            format!(
                "正在分层归并《{title}》知识蓝图（第 {round} 轮，{} 个批次）",
                batches.len()
            ),
        )));
        let mut next = Vec::with_capacity(batches.len());
        for (batch_index, batch) in batches.into_iter().enumerate() {
            cancellation.ensure_active()?;
            let plan = reconcile_once(client, title, &batch, progress, cancellation).await?;
            let mut candidate = ChunkPlan {
                topics: Vec::new(),
                warnings: plan.warnings,
                claims: plan.claims,
                units: plan.units,
            };
            prefix_ids(&mut candidate, &format!("R{round}G{}-", batch_index + 1));
            next.push(candidate);
        }
        candidates = next;
    }
    Err(anyhow!(
        "知识蓝图经过 {MAX_RECONCILE_ROUNDS} 轮仍无法收敛，请缩小单次导入范围"
    ))
}

fn reconcile_batches(candidates: &[ChunkPlan]) -> Vec<Vec<ChunkPlan>> {
    let mut batches = Vec::<Vec<ChunkPlan>>::new();
    let mut current = Vec::new();
    let mut current_chars = 0usize;
    for candidate in candidates {
        let chars = serde_json::to_string(candidate)
            .map(|value| value.chars().count())
            .unwrap_or(MAX_RECONCILE_INPUT_CHARS);
        if !current.is_empty() && current_chars.saturating_add(chars) > MAX_RECONCILE_INPUT_CHARS {
            batches.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push(candidate.clone());
        current_chars = current_chars.saturating_add(chars);
    }
    if !current.is_empty() {
        batches.push(current);
    }

    // Oversized single candidates must still make forward progress. Pairing
    // them may exceed the preferred budget, but remains bounded by two map
    // outputs and halves the candidate count each round.
    if candidates.len() > 1 && batches.len() == candidates.len() {
        return candidates
            .chunks(2)
            .map(|pair| pair.to_vec())
            .collect::<Vec<_>>();
    }
    batches
}

async fn repair_plan(
    client: &ChatClient,
    title: &str,
    plan: &MaterialPlan,
    validation_error: &str,
    material_content: &str,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<MaterialPlan> {
    let input = serde_json::json!({
        "material_title": title,
        "validation_error": validation_error,
        "material_content": bounded_content_sample(material_content, MAX_REPAIR_CONTEXT_CHARS),
        "draft_plan": plan,
    });
    let report_stream = |event| match event {
        StreamEvent::Thinking(text) => progress(ImportEvent::Thinking(text)),
        StreamEvent::Content(text) => progress(ImportEvent::Answer(text)),
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
    prefix_ids(plan, &format!("P{source_index}-"));
}

fn prefix_ids(plan: &mut ChunkPlan, prefix: &str) {
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

fn bounded_content_sample(content: &str, max_chars: usize) -> String {
    let total = content.chars().count();
    if total <= max_chars {
        return content.to_string();
    }
    const WINDOWS: usize = 8;
    let window_chars = max_chars / WINDOWS;
    let characters = content.chars().collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(WINDOWS);
    for index in 0..WINDOWS {
        let center = index * total.saturating_sub(1) / (WINDOWS - 1);
        let start = center
            .saturating_sub(window_chars / 2)
            .min(total - window_chars);
        let end = (start + window_chars).min(total);
        samples.push(characters[start..end].iter().collect::<String>());
    }
    samples.join("\n\n[...省略未采样正文...]\n\n")
}

fn split_plan_chunks(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0;

    for paragraph in content
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let paragraph_chars = paragraph.chars().count();
        if paragraph_chars > MAX_PLAN_CHUNK_CHARS {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_chars = 0;
            }
            let mut piece = String::new();
            for character in paragraph.chars() {
                piece.push(character);
                if piece.chars().count() >= MAX_PLAN_CHUNK_CHARS {
                    chunks.push(std::mem::take(&mut piece));
                }
            }
            if !piece.is_empty() {
                chunks.push(piece);
            }
            continue;
        }

        let separator_chars = if current.is_empty() { 0 } else { 2 };
        if current_chars + separator_chars + paragraph_chars > MAX_PLAN_CHUNK_CHARS
            && !current.is_empty()
        {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
            current_chars += 2;
        }
        current.push_str(paragraph);
        current_chars += paragraph_chars;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        vec![content.to_string()]
    } else {
        chunks
    }
}

fn validate_plan_against_content(mut plan: MaterialPlan, content: &str) -> Result<MaterialPlan> {
    plan = validate_plan(plan)?;
    for claim in &plan.claims {
        for evidence in &claim.evidence {
            if !evidence_matches(content, evidence) {
                return Err(anyhow!(
                    "Claim {} 的证据未出现在清洗后的材料正文中",
                    claim.id
                ));
            }
        }
    }
    for unit in &plan.units {
        for evidence in &unit.evidence {
            if !evidence_matches(content, evidence) {
                return Err(anyhow!(
                    "知识单元 {} 的证据未出现在清洗后的材料正文中",
                    unit.id
                ));
            }
        }
    }
    Ok(plan)
}

fn evidence_matches(content: &str, evidence: &str) -> bool {
    let evidence = evidence.trim();
    if evidence.is_empty() {
        return false;
    }
    content.contains(evidence)
        || normalize_whitespace(content).contains(&normalize_whitespace(evidence))
        || compact_whitespace(content).contains(&compact_whitespace(evidence))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("")
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
        claim.evidence = claim
            .evidence
            .iter()
            .filter_map(|quote| {
                let quote = quote.trim();
                (!quote.is_empty()).then(|| quote.chars().take(MAX_EVIDENCE_CHARS).collect())
            })
            .collect();
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
        unit.evidence = unit
            .evidence
            .iter()
            .filter_map(|quote| {
                let quote = quote.trim();
                (!quote.is_empty()).then(|| quote.chars().take(MAX_EVIDENCE_CHARS).collect())
            })
            .collect();
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
    use super::{
        ChunkPlan, MaterialPlan, PlanClaim, PlanUnit, bounded_content_sample, evidence_matches,
        reconcile_batches, split_plan_chunks, validate_plan, validate_plan_against_content,
    };

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

    #[test]
    fn evidence_must_be_present_in_material_content() {
        assert!(validate_plan_against_content(valid_plan(), "这是原文。其他内容").is_ok());
        assert!(validate_plan_against_content(valid_plan(), "这是另一段内容").is_err());
        assert!(evidence_matches("第一行\n第二行", "第一行 第二行"));
    }

    #[test]
    fn long_materials_are_split_on_paragraph_boundaries() {
        let content = format!("{}\n\n{}", "甲".repeat(40_000), "乙".repeat(40_000));
        let chunks = split_plan_chunks(&content);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), 40_000);
        assert_eq!(chunks[1].chars().count(), 40_000);
    }

    #[test]
    fn reconciliation_batches_shrink_large_candidate_sets() {
        let template = ChunkPlan {
            topics: vec![],
            warnings: vec![],
            claims: vec![],
            units: vec![],
        };
        let candidates = vec![template; 9];
        let batches = reconcile_batches(&candidates);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 9);

        let oversized = ChunkPlan {
            topics: vec!["x".repeat(super::MAX_RECONCILE_INPUT_CHARS)],
            warnings: vec![],
            claims: vec![],
            units: vec![],
        };
        let batches = reconcile_batches(&vec![oversized; 5]);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn repair_context_is_sampled_across_long_content() {
        let content = format!("{}MIDDLE{}", "A".repeat(50_000), "Z".repeat(50_000));
        let sample = bounded_content_sample(&content, 8_000);
        assert!(sample.starts_with('A'));
        assert!(sample.ends_with('Z'));
        assert!(sample.chars().count() < 9_000);
    }
}
