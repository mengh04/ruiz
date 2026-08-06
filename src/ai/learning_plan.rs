use anyhow::{Result, anyhow};
use serde::Deserialize;

use crate::domain::{
    dynamic_review::QuestionFormat,
    knowledge::{KnowledgeUnit, MaterialAnalysis},
    learning::{
        ContentBlock, LearningIntent, LearningPlan, LearningStep, LearningStepKind, LearningTopic,
        UnitSourceLink, validate_plan,
    },
    note::Note,
};

use super::{client::ChatClient, prompts::LEARNING_PLAN_SYSTEM};

#[derive(Deserialize)]
struct Response {
    plan_version: i64,
    summary: String,
    estimated_minutes: usize,
    topics: Vec<LearningTopic>,
    steps: Vec<ResponseStep>,
}

#[derive(Deserialize)]
struct ResponseStep {
    id: String,
    kind: LearningStepKind,
    topic_id: String,
    #[serde(default)]
    block_ids: Vec<String>,
    #[serde(default)]
    unit_ids: Vec<String>,
    #[serde(default)]
    source_step_ids: Vec<String>,
    intent: Option<LearningIntent>,
    #[serde(rename = "format")]
    question_format: Option<QuestionFormat>,
    reason: Option<String>,
}

pub async fn generate(
    client: &ChatClient,
    note: &Note,
    analysis: Option<&MaterialAnalysis>,
    blocks: &[ContentBlock],
    units: &[KnowledgeUnit],
    links: &[UnitSourceLink],
) -> Result<LearningPlan> {
    let input = serde_json::json!({
        "material": {
            "title": note.title,
            "summary": analysis.map(|value| value.summary.as_str()).unwrap_or(""),
            "document_type": analysis.map(|value| value.document_type.as_str()).unwrap_or("unknown"),
        },
        "content_blocks": blocks.iter().map(|block| serde_json::json!({
            "id": block.local_id,
            "kind": block.kind,
            "heading_path": block.heading_path,
            "text": block.source_text,
        })).collect::<Vec<_>>(),
        "knowledge_units": units.iter().map(|unit| serde_json::json!({
            "id": unit.local_id,
            "topic": unit.topic,
            "objective": unit.objective,
            "stage": unit.stage,
            "cognitive_action": unit.cognitive_action,
            "required_points": unit.required_points,
            "recommended": unit.recommended,
            "prerequisite_ids": unit.prerequisite_unit_ids,
        })).collect::<Vec<_>>(),
        "source_links": links,
    });
    let value = client
        .chat_json_for(
            "learning.plan.generate",
            LEARNING_PLAN_SYSTEM,
            &input.to_string(),
        )
        .await?;
    let response: Response =
        serde_json::from_value(value).map_err(|error| anyhow!("学习路线响应格式不对: {error}"))?;
    let topics = response.topics;
    let steps = response
        .steps
        .into_iter()
        .enumerate()
        .map(|(position, step)| {
            let topic_title = topics
                .iter()
                .find(|topic| topic.id == step.topic_id)
                .map(|topic| topic.title.clone())
                .unwrap_or_default();
            LearningStep {
                id: None,
                local_id: step.id,
                topic_id: step.topic_id,
                topic_title,
                kind: step.kind,
                block_ids: step.block_ids,
                unit_ids: step.unit_ids,
                source_step_ids: step.source_step_ids,
                intent: step.intent,
                question_format: step.question_format,
                reason: step.reason,
                position,
            }
        })
        .collect();
    let plan = LearningPlan {
        id: None,
        note_id: note.id,
        content_hash: blocks
            .first()
            .map(|block| block.content_hash.clone())
            .unwrap_or_default(),
        plan_version: response.plan_version,
        summary: response.summary.trim().to_string(),
        estimated_minutes: response.estimated_minutes.max(1),
        generation_mode: "ai".into(),
        topics,
        steps,
    };
    if plan.summary.is_empty() {
        return Err(anyhow!("AI 返回了空的学习路线说明"));
    }
    validate_plan(&plan, blocks, units)?;
    Ok(plan)
}
