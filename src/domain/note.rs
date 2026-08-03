use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// AI 从一次原始粘贴中整理出的单篇学习材料。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    /// AI 根据正文生成的材料标题。
    pub title: String,
    /// 去除网页外壳、重复目录等噪声后的正文。
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
