use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 对一次作答的评分，对应 FSRS 的 1-4 档（Anki 的 Again/Hard/Good/Easy）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    /// 由 FSRS 的 1-4 数值档位转换为评分。
    #[allow(dead_code)] // 对外转换 API（db 层使用）
    pub fn from_fsrs(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Again),
            2 => Some(Self::Hard),
            3 => Some(Self::Good),
            4 => Some(Self::Easy),
            _ => None,
        }
    }
}

/// 一次答题记录：用户的答案 + AI 判官的反馈 + 评分。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: i64,
    pub card_id: i64,
    pub user_answer: String,
    /// AI 判官给出的建议（为什么对/错、怎么改进）
    pub ai_feedback: String,
    pub rating: Rating,
    pub reviewed_at: DateTime<Utc>,
}
