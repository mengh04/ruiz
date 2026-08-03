use chrono::{DateTime, Utc};
use fsrs::MemoryState;

/// 一张卡片：一个「问题 + 标准答案 + 原文引用」，
/// 附带 FSRS 记忆状态用于间隔重复调度。
#[derive(Debug, Clone)]
pub struct Card {
    pub id: i64,
    pub note_id: i64,
    pub question: String,
    pub standard_answer: String,
    /// 原文中与本题相关的片段（可选）
    pub source_excerpt: Option<String>,
    /// FSRS 记忆状态；`None` 表示新卡（从未复习过）
    pub memory: Option<MemoryState>,
    /// 下次到期时间（复习队列按此排序）
    pub due: DateTime<Utc>,
    /// 累计复习次数
    pub reps: u32,
    /// 累计遗忘（Again）次数
    pub lapses: u32,
    /// 上次复习时间（新卡为 None）
    pub last_review: Option<DateTime<Utc>>,
    /// 元数据（当前 UI 未展示，保留用于数据完整性）
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

impl Card {
    /// 由 AI 出题结果构造一张新卡（id=0 表示尚未入库）
    pub fn new(
        note_id: i64,
        question: String,
        standard_answer: String,
        source_excerpt: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: 0,
            note_id,
            question,
            standard_answer,
            source_excerpt,
            memory: None,
            due: now,
            reps: 0,
            lapses: 0,
            last_review: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// 距上次复习的天数（FSRS 的 delta_t；新卡返回 0）
    pub fn days_elapsed(&self, now: DateTime<Utc>) -> u32 {
        match self.last_review {
            Some(last) => (now - last).num_days().max(0) as u32,
            None => 0,
        }
    }
}
