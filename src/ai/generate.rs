use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{client::ChatClient, prompts::GENERATE_SYSTEM};

/// AI 出题得到的一道题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub standard_answer: String,
    /// 原文中与本题直接相关的片段（可选，答题时可对照原文）
    #[serde(default)]
    pub source_excerpt: Option<String>,
}

/// 让 AI 把学习材料拆成 `count` 道题。
pub async fn generate_questions(
    client: &ChatClient,
    content: &str,
    count: usize,
) -> Result<Vec<Question>> {
    let user = serde_json::json!({
        "requested_count": count,
        "material": content,
    })
    .to_string();
    let json = client.chat_json(GENERATE_SYSTEM, &user).await?;
    let questions = json
        .get("questions")
        .ok_or_else(|| anyhow!("出题响应缺少 questions 字段：{json}"))?;
    let list: Vec<Question> =
        serde_json::from_value(questions.clone()).map_err(|e| anyhow!("出题响应格式不对: {e}"))?;
    if list.is_empty() {
        return Err(anyhow!("AI 没有生成任何题目"));
    }
    Ok(list)
}
