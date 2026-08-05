use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 一组相关学习材料（类似 Anki 的牌组）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyGroup {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 分组概览，用于资料库和复习页展示进度。
#[derive(Debug, Clone)]
pub struct GroupSummary {
    pub group: StudyGroup,
    pub note_count: usize,
    pub card_count: usize,
    pub due_count: usize,
}
