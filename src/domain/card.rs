use chrono::{DateTime, Utc};

/// 一道兼容旧版数据的基础题：一个「问题 + 标准答案 + 原文引用」。
/// 动态复习以关联知识单元上的 FSRS 状态为准。
#[derive(Debug, Clone)]
pub struct Card {
    #[allow(dead_code)]
    pub note_id: i64,
    pub question: String,
    pub standard_answer: String,
    /// 原文中与本题相关的片段（可选）
    pub source_excerpt: Option<String>,
    /// 基础题关联的知识单元；旧数据会在数据库迁移时自动补建关联。
    #[allow(dead_code)]
    pub knowledge_unit_id: Option<i64>,
    /// 下次到期时间（复习队列按此排序）
    #[allow(dead_code)]
    pub due: DateTime<Utc>,
    /// 累计复习次数
    #[allow(dead_code)]
    pub reps: u32,
    /// 累计遗忘（Again）次数
    #[allow(dead_code)]
    pub lapses: u32,
    /// 元数据（当前 UI 未展示，保留用于数据完整性）
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

impl Card {
    /// 由 AI 出题结果构造一张新卡（id=0 表示尚未入库）
    #[allow(dead_code)]
    pub fn new(
        note_id: i64,
        question: String,
        standard_answer: String,
        source_excerpt: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            note_id,
            question,
            standard_answer,
            source_excerpt,
            knowledge_unit_id: None,
            due: now,
            reps: 0,
            lapses: 0,
            created_at: now,
            updated_at: now,
        }
    }
}
