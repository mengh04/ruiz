use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{
    client::{ChatClient, StreamEvent},
    plan::PlanUnit,
    progress::{ImportCancellation, ImportEvent, ImportEventReporter, ImportProgress, ImportStage},
    prompts::GENERATE_SYSTEM,
};

const MAX_GENERATION_UNITS: usize = 40;

/// AI 出题得到的一道题，以及它对应的知识单元和必答点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub unit_id: String,
    pub question: String,
    pub standard_answer: String,
    #[serde(default)]
    pub source_excerpt: Option<String>,
    #[serde(default)]
    pub required_points: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct QuestionResponse {
    questions: Vec<Question>,
}

/// 为已经选定的知识单元逐一生成问题，不再要求模型凑固定题数。
pub async fn generate_questions(client: &ChatClient, units: &[PlanUnit]) -> Result<Vec<Question>> {
    let silent = |_: ImportEvent| {};
    let cancellation = ImportCancellation::default();
    generate_questions_with_progress(client, units, "当前材料", &silent, &cancellation).await
}

pub async fn generate_questions_with_progress(
    client: &ChatClient,
    units: &[PlanUnit],
    material_title: &str,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<Vec<Question>> {
    cancellation.ensure_active()?;
    if units.is_empty() {
        return Ok(Vec::new());
    }
    let batch_count = units.len().div_ceil(MAX_GENERATION_UNITS);
    let mut generated_questions = Vec::with_capacity(units.len());
    for (batch_index, batch) in units.chunks(MAX_GENERATION_UNITS).enumerate() {
        cancellation.ensure_active()?;
        progress(ImportEvent::Stage(ImportProgress::stage(
            ImportStage::Generating,
            format!(
                "正在为《{material_title}》生成基础题（第 {}/{} 批，共 {} 道）",
                batch_index + 1,
                batch_count,
                units.len()
            ),
        )));
        let input = serde_json::json!({
            "batch_index": batch_index + 1,
            "batch_count": batch_count,
            "units": batch,
        });
        let report_stream = |event| match event {
            StreamEvent::Thinking(text) => progress(ImportEvent::Thinking(text)),
            StreamEvent::Content(text) => progress(ImportEvent::Answer(text)),
        };
        let value = client
            .chat_json_stream_for(
                "questions.generate",
                GENERATE_SYSTEM,
                &input.to_string(),
                &report_stream,
                Some(cancellation),
            )
            .await?;
        let response: QuestionResponse = serde_json::from_value(value)
            .map_err(|error| anyhow!("出题响应格式不对（第 {} 批）: {error}", batch_index + 1))?;
        validate_questions(batch, &response.questions)?;
        generated_questions.extend(response.questions);
    }
    validate_questions(units, &generated_questions)?;
    let mut generated = HashMap::<String, Question>::new();
    for mut question in generated_questions {
        let unit = units
            .iter()
            .find(|unit| unit.id == question.unit_id)
            .expect("响应校验后知识单元应存在");
        question.question = question.question.trim().to_string();
        question.standard_answer = question.standard_answer.trim().to_string();
        // 证据由经过校验的知识蓝图提供，不信任生成阶段重新抄写的引用。
        question.source_excerpt = Some(unit.evidence.join("\n"));
        question.required_points = unit.required_points.clone();
        if generated
            .insert(question.unit_id.clone(), question)
            .is_some()
        {
            return Err(anyhow!("AI 为同一知识单元重复生成了题目"));
        }
    }
    units
        .iter()
        .map(|unit| {
            generated
                .remove(&unit.id)
                .ok_or_else(|| anyhow!("AI 没有为知识单元 {} 生成题目", unit.id))
        })
        .collect()
}

fn validate_questions(units: &[PlanUnit], questions: &[Question]) -> Result<()> {
    if units.len() != questions.len() {
        return Err(anyhow!(
            "AI 应生成 {} 道题，实际返回 {} 道",
            units.len(),
            questions.len()
        ));
    }
    let expected = units
        .iter()
        .map(|unit| unit.id.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for question in questions {
        if !expected.contains(question.unit_id.as_str()) {
            return Err(anyhow!("AI 返回了未知知识单元: {}", question.unit_id));
        }
        if !seen.insert(question.unit_id.as_str()) {
            return Err(anyhow!("AI 重复返回知识单元: {}", question.unit_id));
        }
        if question.question.trim().is_empty() || question.standard_answer.trim().is_empty() {
            return Err(anyhow!("AI 返回了空问题或空标准答案"));
        }
    }
    Ok(())
}
