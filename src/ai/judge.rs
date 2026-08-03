use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{client::ChatClient, prompts::JUDGE_SYSTEM};

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

/// 让 AI 评判用户的一次作答。
pub async fn judge(
    client: &ChatClient,
    question: &str,
    standard_answer: &str,
    user_answer: &str,
) -> Result<Judgement> {
    let user = serde_json::json!({
        "question": question,
        "standard_answer": standard_answer,
        "user_answer": user_answer,
    })
    .to_string();
    let json = client.chat_json(JUDGE_SYSTEM, &user).await?;
    let judgement: Judgement =
        serde_json::from_value(json).map_err(|e| anyhow!("判官响应格式不对: {e}"))?;
    if !matches!(
        judgement.rating.as_str(),
        "again" | "hard" | "good" | "easy"
    ) {
        return Err(anyhow!("判官返回了非法评分: {}", judgement.rating));
    }
    if judgement.score > 100 {
        return Err(anyhow!("判官返回了非法分数: {}", judgement.score));
    }
    if judgement.feedback.trim().is_empty() {
        return Err(anyhow!("判官返回了空反馈"));
    }
    if !score_matches_rating(judgement.score, &judgement.rating) {
        return Err(anyhow!(
            "判官返回的评分档位与分数不一致: {} / {}",
            judgement.rating,
            judgement.score
        ));
    }
    Ok(judgement)
}

fn score_matches_rating(score: u32, rating: &str) -> bool {
    match rating {
        "again" => score <= 49,
        "hard" => (50..=74).contains(&score),
        "good" => (75..=94).contains(&score),
        "easy" => (95..=100).contains(&score),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::score_matches_rating;

    #[test]
    fn score_bands_match_review_ratings() {
        assert!(score_matches_rating(0, "again"));
        assert!(score_matches_rating(49, "again"));
        assert!(score_matches_rating(50, "hard"));
        assert!(score_matches_rating(74, "hard"));
        assert!(score_matches_rating(75, "good"));
        assert!(score_matches_rating(94, "good"));
        assert!(score_matches_rating(95, "easy"));
        assert!(score_matches_rating(100, "easy"));

        assert!(!score_matches_rating(70, "good"));
        assert!(!score_matches_rating(85, "hard"));
        assert!(!score_matches_rating(101, "easy"));
    }
}
