use std::collections::HashSet;

use anyhow::{Result, anyhow};
use serde::Deserialize;

use crate::domain::{
    dynamic_review::QuestionFormat,
    knowledge::KnowledgeUnit,
    learning::{ContentBlock, LearningPrompt, LearningStep},
};

use super::{client::ChatClient, prompts::LEARNING_QUESTION_SYSTEM};

#[derive(Deserialize)]
struct Response {
    covered_unit_ids: Vec<String>,
    question_type: QuestionFormat,
    question: String,
    #[serde(default)]
    options: Vec<String>,
    standard_answer: String,
    required_points: Vec<String>,
}

pub struct QuestionContext<'a> {
    pub target_unit_ids: &'a [String],
    pub position: usize,
    pub total_questions: usize,
    pub recent_questions: &'a [String],
}

pub async fn generate(
    client: &ChatClient,
    step: &LearningStep,
    units: &[KnowledgeUnit],
    blocks: &[ContentBlock],
    context: QuestionContext<'_>,
) -> Result<LearningPrompt> {
    let step_id = step.id.ok_or_else(|| anyhow!("学习步骤尚未持久化"))?;
    let selected_units = units
        .iter()
        .filter(|unit| context.target_unit_ids.contains(&unit.local_id))
        .collect::<Vec<_>>();
    if selected_units.is_empty() {
        return Err(anyhow!("学习题没有考察目标"));
    }
    let requested = question_format_for_units(step, &selected_units);
    let source_ids = blocks
        .iter()
        .filter(|block| {
            selected_units.iter().any(|unit| {
                unit.evidence
                    .iter()
                    .any(|evidence| block.plain_text.contains(evidence))
            })
        })
        .map(|block| block.local_id.clone())
        .collect::<Vec<_>>();
    let source_blocks = blocks
        .iter()
        .filter(|block| source_ids.is_empty() || source_ids.contains(&block.local_id))
        .take(8)
        .map(|block| serde_json::json!({"block_id": block.local_id, "text": block.source_text}))
        .collect::<Vec<_>>();
    let input = serde_json::json!({
        "intent": step.intent,
        "requested_question_type": requested,
        "knowledge_units": selected_units,
        "source_blocks": source_blocks,
        "question_position": context.position + 1,
        "total_questions": context.total_questions,
        "recent_questions": context.recent_questions,
    });
    let value = client
        .chat_json_for(
            "learning.question.generate",
            LEARNING_QUESTION_SYSTEM,
            &input.to_string(),
        )
        .await?;
    let response: Response =
        serde_json::from_value(value).map_err(|error| anyhow!("学习题响应格式不对: {error}"))?;
    validate(requested, context.target_unit_ids, &response)?;
    Ok(LearningPrompt {
        id: None,
        learning_step_id: step_id,
        position: context.position,
        unit_ids: context.target_unit_ids.to_vec(),
        format: requested,
        question: response.question.trim().into(),
        options: response
            .options
            .into_iter()
            .map(|value| value.trim().into())
            .collect(),
        standard_answer: response.standard_answer.trim().into(),
        required_points: response
            .required_points
            .into_iter()
            .map(|value| value.trim().into())
            .collect(),
        source_block_ids: source_ids,
        generation_mode: "ai_v3".into(),
    })
}

fn question_format_for_units(step: &LearningStep, units: &[&KnowledgeUnit]) -> QuestionFormat {
    if units.len() > 1 {
        if units.iter().any(|unit| {
            matches!(
                unit.cognitive_action.as_str(),
                "diagnose" | "decide" | "predict"
            )
        }) {
            QuestionFormat::Application
        } else {
            QuestionFormat::ShortAnswer
        }
    } else {
        match units[0].cognitive_action.as_str() {
            "recall" => step.question_format.unwrap_or(QuestionFormat::Choice),
            "diagnose" | "decide" | "predict" => QuestionFormat::Application,
            _ => QuestionFormat::ShortAnswer,
        }
    }
}

fn validate(
    expected: QuestionFormat,
    target_unit_ids: &[String],
    response: &Response,
) -> Result<()> {
    let expected_units = target_unit_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let covered_units = response
        .covered_unit_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if expected_units != covered_units || covered_units.len() != response.covered_unit_ids.len() {
        return Err(anyhow!("学习题没有准确覆盖指定知识单元"));
    }
    if response.question_type != expected
        || response.question.trim().is_empty()
        || response.standard_answer.trim().is_empty()
    {
        return Err(anyhow!("AI 返回的题型不一致或题面为空"));
    }
    if response.required_points.is_empty()
        || response
            .required_points
            .iter()
            .any(|point| point.trim().is_empty())
    {
        return Err(anyhow!("学习题必须包含非空必答点"));
    }
    if expected == QuestionFormat::Choice {
        let options = response
            .options
            .iter()
            .map(|value| value.trim())
            .collect::<Vec<_>>();
        if !(3..=5).contains(&options.len())
            || options.iter().copied().collect::<HashSet<_>>().len() != options.len()
            || !options.contains(&response.standard_answer.trim())
        {
            return Err(anyhow!("选择题选项不符合契约"));
        }
    } else if !response.options.is_empty() {
        return Err(anyhow!("非选择题不应返回选项"));
    }
    if expected == QuestionFormat::FillBlank && !response.question.contains("____") {
        return Err(anyhow!("填空题缺少空白标记"));
    }
    if reveals_answer(response) {
        return Err(anyhow!("学习题题面泄露了答案或判分要点"));
    }
    Ok(())
}

fn reveals_answer(response: &Response) -> bool {
    let question = response.question.trim();
    let answer = response.standard_answer.trim();
    (answer.chars().count() >= 8 && question.contains(answer))
        || response.required_points.iter().any(|point| {
            let point = point.trim();
            point.chars().count() >= 12 && question.contains(point)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_statements_are_rejected_as_questions() {
        let response = Response {
            covered_unit_ids: vec!["K1".into()],
            question_type: QuestionFormat::ShortAnswer,
            question: "请解释这个关键点：主从部署保持主从数据同步。".into(),
            options: Vec::new(),
            standard_answer: "主从部署保持主从数据同步。".into(),
            required_points: vec!["主从部署保持主从数据同步。".into()],
        };
        assert!(reveals_answer(&response));
    }
}
