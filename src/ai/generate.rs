use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::client::ChatClient;

/// AI 出题得到的一道题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub standard_answer: String,
    /// 原文中与本题直接相关的片段（可选，答题时可对照原文）
    #[serde(default)]
    pub source_excerpt: Option<String>,
}

const GENERATE_SYSTEM: &str = r#"你是严谨的出题老师。用户会提供一份学习材料，你的任务是把材料拆解成若干道高质量的简答题/问答题，覆盖材料中的关键知识点。

要求：
1. 问题要具体、可独立作答，不要问"什么是XX"这种泛泛的问题，要能考察真正的理解。
2. 标准答案要准确、完整，来自材料内容，不要编造材料外的知识。
3. source_excerpt 填该题对应的原文片段（原文里连续的一段），方便用户对照；找不到合适的可以留 null。
4. 输出严格的 JSON 对象，格式如下，不要输出任何其他内容：
{"questions":[{"question":"...","standard_answer":"...","source_excerpt":"..."}]}"#;

/// 让 AI 把学习材料拆成 `count` 道题。
pub async fn generate_questions(
    client: &ChatClient,
    content: &str,
    count: usize,
) -> Result<Vec<Question>> {
    let user = format!("请针对以下学习材料出 {count} 道题：\n\n---\n{content}\n---");
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
