use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::client::ChatClient;

/// AI 判官的裁决结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgement {
    /// 评分档位：`again` / `hard` / `good` / `easy`
    pub rating: String,
    /// 0-100 分
    pub score: u32,
    /// 给用户的建议：哪里答得好、哪里遗漏、标准答案的关键点
    pub feedback: String,
}

const JUDGE_SYSTEM: &str = r#"你是严格的面试判官。用户会给你一道题、标准答案和用户的作答，你要像面试官一样评判。

要求：
1. 客观评分：对照标准答案判断用户答案的正确性和完整性。
2. rating 四选一：
   - "again"：基本没答对 / 严重遗漏，需要重学
   - "hard"：答对一部分但关键点缺失或错误
   - "good"：基本答对，只差少量细节
   - "easy"：答得很完整准确，超出了标准答案的要点
3. feedback 用简洁中文给出具体建议：指出遗漏/错误的关键点，以及如何改进。不要空话。
4. 输出严格的 JSON 对象：{"rating":"good","score":85,"feedback":"..."}，不要输出任何其他内容。"#;

/// 让 AI 评判用户的一次作答。
pub async fn judge(
    client: &ChatClient,
    question: &str,
    standard_answer: &str,
    user_answer: &str,
) -> Result<Judgement> {
    let user =
        format!("题目：{question}\n\n标准答案：{standard_answer}\n\n用户的作答：\n{user_answer}");
    let json = client.chat_json(JUDGE_SYSTEM, &user).await?;
    let judgement: Judgement =
        serde_json::from_value(json).map_err(|e| anyhow!("判官响应格式不对: {e}"))?;
    if !matches!(
        judgement.rating.as_str(),
        "again" | "hard" | "good" | "easy"
    ) {
        return Err(anyhow!("判官返回了非法评分: {}", judgement.rating));
    }
    Ok(judgement)
}
