use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::collections::HashSet;

use crate::domain::dynamic_review::{QuestionFormat, ReviewItem, ReviewPrompt};

use super::{client::ChatClient, prompts::DYNAMIC_QUESTION_SYSTEM};

#[derive(Debug, Deserialize)]
struct DynamicQuestionResponse {
    question_type: QuestionFormat,
    question: String,
    #[serde(default)]
    options: Vec<String>,
    standard_answer: String,
}

pub async fn generate(
    client: &ChatClient,
    item: &ReviewItem,
    format: QuestionFormat,
    recent_questions: &[String],
) -> Result<ReviewPrompt> {
    let mastery = item.mastery_band();
    let input = serde_json::json!({
        "knowledge_unit": {
            "topic": item.topic,
            "objective": item.objective,
            "unit_type": item.unit_type,
            "cognitive_action": item.cognitive_action,
            "required_points": item.required_points,
            "evidence": item.evidence,
        },
        "requested_question_type": format,
        "mastery_band": mastery.as_str(),
        "recent_questions": recent_questions,
    });
    let value = client
        .chat_json_for(
            "review.question.generate",
            DYNAMIC_QUESTION_SYSTEM,
            &input.to_string(),
        )
        .await?;
    let response: DynamicQuestionResponse =
        serde_json::from_value(value).map_err(|error| anyhow!("动态出题响应格式不对: {error}"))?;
    validate(format, &response)?;
    Ok(ReviewPrompt {
        id: None,
        unit_id: item.unit_id,
        format,
        mastery,
        question: response.question.trim().to_string(),
        options: response
            .options
            .into_iter()
            .map(|option| option.trim().to_string())
            .collect(),
        standard_answer: response.standard_answer.trim().to_string(),
        required_points: item.required_points.clone(),
        source_excerpt: Some(item.evidence.join("\n")),
        generation_mode: "ai".into(),
    })
}

fn validate(expected: QuestionFormat, response: &DynamicQuestionResponse) -> Result<()> {
    if response.question_type != expected {
        return Err(anyhow!("AI 返回的题型与请求题型不一致"));
    }
    if response.question.trim().is_empty() || response.standard_answer.trim().is_empty() {
        return Err(anyhow!("AI 返回了空问题或空标准答案"));
    }
    if expected == QuestionFormat::Choice {
        if !(3..=5).contains(&response.options.len()) {
            return Err(anyhow!("选择题必须返回 3-5 个选项"));
        }
        let normalized_options = response
            .options
            .iter()
            .map(|option| option.trim())
            .collect::<Vec<_>>();
        if normalized_options.iter().any(|option| option.is_empty()) {
            return Err(anyhow!("选择题选项不能为空"));
        }
        if normalized_options
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            != normalized_options.len()
        {
            return Err(anyhow!("选择题选项不能重复"));
        }
        let answer = response.standard_answer.trim();
        if !normalized_options.contains(&answer) {
            return Err(anyhow!("选择题标准答案必须与其中一个选项完全一致"));
        }
    } else if !response.options.is_empty() {
        return Err(anyhow!("非选择题不应返回选项"));
    }
    if expected == QuestionFormat::FillBlank
        && !response.question.contains("____")
        && !response.question.contains("___")
    {
        return Err(anyhow!("填空题题面必须包含空白标记"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_choice_contract() {
        let valid = DynamicQuestionResponse {
            question_type: QuestionFormat::Choice,
            question: "哪一项正确？".into(),
            options: vec!["A".into(), "B".into(), "C".into()],
            standard_answer: "B".into(),
        };
        assert!(validate(QuestionFormat::Choice, &valid).is_ok());
        let mut invalid = valid;
        invalid.standard_answer = "D".into();
        assert!(validate(QuestionFormat::Choice, &invalid).is_err());

        let duplicate = DynamicQuestionResponse {
            question_type: QuestionFormat::Choice,
            question: "哪一项正确？".into(),
            options: vec!["A".into(), "A".into(), "C".into()],
            standard_answer: "A".into(),
        };
        assert!(validate(QuestionFormat::Choice, &duplicate).is_err());
    }
}
