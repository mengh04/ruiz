//! FSRS 间隔重复调度封装（fsrs crate v6）。

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use fsrs::{FSRS, ItemState, MemoryState, NextStates};

use crate::domain::review::Rating;

/// 目标记忆保持率（Anki 默认 0.9 = 到期时期望仍有 90% 回忆率）
pub const DESIRED_RETENTION: f32 = 0.9;

pub struct Scheduler {
    fsrs: FSRS,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            fsrs: FSRS::new(&[]).expect("默认 FSRS 参数应当有效"),
        }
    }

    /// 计算一次复习后 4 个评分档位各自对应的下一状态。
    /// `memory: None` 表示新卡；`days_elapsed` 为距上次复习的天数（新卡传 0）。
    pub fn next_states(
        &self,
        memory: Option<MemoryState>,
        days_elapsed: u32,
    ) -> Result<NextStates> {
        Ok(self
            .fsrs
            .next_states(memory, DESIRED_RETENTION, days_elapsed)?)
    }

    /// 按评分档位取出对应状态。
    pub fn state_for(rating: Rating, next: &NextStates) -> &ItemState {
        match rating {
            Rating::Again => &next.again,
            Rating::Hard => &next.hard,
            Rating::Good => &next.good,
            Rating::Easy => &next.easy,
        }
    }

    /// 由 FSRS 算出的 interval（天）换算成下次到期时间。
    pub fn due_date(state: &ItemState) -> DateTime<Utc> {
        let millis = (state.interval * 86_400_000.0) as i64;
        Utc::now() + Duration::milliseconds(millis.max(0))
    }
}
