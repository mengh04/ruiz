use chrono::{DateTime, Utc};
use fsrs::{FSRS6_DEFAULT_DECAY, MemoryState, current_retrievability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionFormat {
    Choice,
    FillBlank,
    ShortAnswer,
    Application,
}

impl QuestionFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Choice => "选择题",
            Self::FillBlank => "填空题",
            Self::ShortAnswer => "简答题",
            Self::Application => "应用题",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::FillBlank => "fill_blank",
            Self::ShortAnswer => "short_answer",
            Self::Application => "application",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "choice" => Ok(Self::Choice),
            "fill_blank" => Ok(Self::FillBlank),
            "short_answer" => Ok(Self::ShortAnswer),
            "application" => Ok(Self::Application),
            _ => Err(anyhow::anyhow!("未知题型: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasteryBand {
    Beginner,
    Developing,
    Strong,
}

/// 知识蓝图环形熟练度使用的颜色分级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProficiencyLevel {
    Unassessed,
    Low,
    Medium,
    High,
}

/// 知识单元的 FSRS 记忆状态，同时供复习调度和知识蓝图展示使用。
#[derive(Debug, Clone, Default)]
pub struct ReviewState {
    pub memory: Option<MemoryState>,
    pub reps: u32,
    pub lapses: u32,
    pub last_review: Option<DateTime<Utc>>,
}

impl ReviewState {
    pub fn days_elapsed(&self, now: DateTime<Utc>) -> u32 {
        self.last_review
            .map(|last| (now - last).num_days().max(0) as u32)
            .unwrap_or(0)
    }

    pub fn mastery_band(&self) -> MasteryBand {
        let unstable = self
            .memory
            .is_some_and(|memory| memory.stability < 7.0 || memory.difficulty > 7.0);
        if self.reps <= 1 || self.lapses.saturating_mul(2) >= self.reps.max(1) {
            MasteryBand::Beginner
        } else if self.reps < 5 || unstable {
            MasteryBand::Developing
        } else {
            MasteryBand::Strong
        }
    }

    /// 百分制熟练度同时考虑此刻能否回忆和记忆能保持多久。
    /// 稳定性每增加 7 天完成一半剩余成长，避免刚复习后的回忆率虚高。
    pub fn proficiency(&self, now: DateTime<Utc>) -> Option<f32> {
        if self.reps == 0 || self.memory.is_none() || self.last_review.is_none() {
            return None;
        }
        let memory = self.memory?;
        let retrievability = self.retrievability(now)?;
        let durability = 1.0 - 2.0_f32.powf(-memory.stability.max(0.0) / 7.0);
        Some((retrievability * durability * 100.0).clamp(0.0, 100.0))
    }

    pub fn proficiency_level(&self, now: DateTime<Utc>) -> ProficiencyLevel {
        match self.proficiency(now) {
            None => ProficiencyLevel::Unassessed,
            Some(score) if score < 40.0 => ProficiencyLevel::Low,
            Some(score) if score < 75.0 => ProficiencyLevel::Medium,
            Some(_) => ProficiencyLevel::High,
        }
    }

    /// 基于 FSRS-6 遗忘曲线计算此刻成功回忆的概率。
    pub fn retrievability(&self, now: DateTime<Utc>) -> Option<f32> {
        let memory = self.memory?;
        let last_review = self.last_review?;
        let elapsed_days = (now - last_review).num_seconds().max(0) as f32 / 86_400.0;
        Some(current_retrievability(memory, elapsed_days, FSRS6_DEFAULT_DECAY).clamp(0.0, 1.0))
    }
}

impl MasteryBand {
    pub fn label(self) -> &'static str {
        match self {
            Self::Beginner => "基础巩固",
            Self::Developing => "主动回忆",
            Self::Strong => "迁移应用",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Developing => "developing",
            Self::Strong => "strong",
        }
    }
}

/// FSRS 调度的稳定对象，一条记录对应一个知识单元。
#[derive(Debug, Clone)]
pub struct ReviewItem {
    pub unit_id: i64,
    pub note_title: String,
    pub topic: String,
    pub objective: String,
    pub unit_type: String,
    pub cognitive_action: String,
    pub required_points: Vec<String>,
    pub evidence: Vec<String>,
    pub seed_card_id: Option<i64>,
    pub fallback_question: Option<String>,
    pub fallback_answer: Option<String>,
    pub fallback_source: Option<String>,
    pub memory: Option<MemoryState>,
    pub reps: u32,
    pub lapses: u32,
    pub last_review: Option<DateTime<Utc>>,
}

impl ReviewItem {
    pub fn days_elapsed(&self, now: DateTime<Utc>) -> u32 {
        self.review_state().days_elapsed(now)
    }

    pub fn mastery_band(&self) -> MasteryBand {
        self.review_state().mastery_band()
    }

    pub fn review_state(&self) -> ReviewState {
        ReviewState {
            memory: self.memory,
            reps: self.reps,
            lapses: self.lapses,
            last_review: self.last_review,
        }
    }

    pub fn next_question_format(&self) -> QuestionFormat {
        let attempt = (self.reps + self.lapses) as usize;
        match self.mastery_band() {
            MasteryBand::Beginner => {
                [QuestionFormat::Choice, QuestionFormat::FillBlank][attempt % 2]
            }
            MasteryBand::Developing => [
                QuestionFormat::FillBlank,
                QuestionFormat::ShortAnswer,
                QuestionFormat::Choice,
            ][attempt % 3],
            MasteryBand::Strong => [
                QuestionFormat::ShortAnswer,
                QuestionFormat::Application,
                QuestionFormat::FillBlank,
            ][attempt % 3],
        }
    }

    pub fn question_format(&self, adaptive_answer_formats: bool) -> QuestionFormat {
        if adaptive_answer_formats {
            self.next_question_format()
        } else {
            QuestionFormat::ShortAnswer
        }
    }

    pub fn fallback_prompt(&self) -> ReviewPrompt {
        ReviewPrompt {
            id: None,
            unit_id: self.unit_id,
            format: QuestionFormat::ShortAnswer,
            mastery: self.mastery_band(),
            question: self
                .fallback_question
                .clone()
                .unwrap_or_else(|| format!("请完成这个学习目标：{}", self.objective)),
            options: Vec::new(),
            standard_answer: self
                .fallback_answer
                .clone()
                .unwrap_or_else(|| self.required_points.join("；")),
            required_points: self.required_points.clone(),
            source_excerpt: self
                .fallback_source
                .clone()
                .or_else(|| Some(self.evidence.join("\n"))),
            generation_mode: "fallback".into(),
        }
    }
}

/// 一次实际展示给用户的题面快照。
#[derive(Debug, Clone)]
pub struct ReviewPrompt {
    pub id: Option<i64>,
    pub unit_id: i64,
    pub format: QuestionFormat,
    pub mastery: MasteryBand,
    pub question: String,
    pub options: Vec<String>,
    pub standard_answer: String,
    pub required_points: Vec<String>,
    pub source_excerpt: Option<String>,
    pub generation_mode: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn item(reps: u32, lapses: u32, memory: Option<MemoryState>) -> ReviewItem {
        ReviewItem {
            unit_id: 1,
            note_title: "章节".into(),
            topic: "主题".into(),
            objective: "目标".into(),
            unit_type: "concept".into(),
            cognitive_action: "recall".into(),
            required_points: vec!["要点".into()],
            evidence: vec!["证据".into()],
            seed_card_id: None,
            fallback_question: None,
            fallback_answer: None,
            fallback_source: None,
            memory,
            reps,
            lapses,
            last_review: None,
        }
    }

    #[test]
    fn question_format_adapts_to_mastery() {
        assert_eq!(
            item(0, 0, None).question_format(false),
            QuestionFormat::ShortAnswer
        );
        assert_eq!(
            item(0, 0, None).question_format(true),
            QuestionFormat::Choice
        );
        assert_eq!(
            item(1, 0, None).question_format(true),
            QuestionFormat::FillBlank
        );
        assert_eq!(
            item(
                6,
                0,
                Some(MemoryState {
                    stability: 20.0,
                    difficulty: 4.0,
                })
            )
            .question_format(true),
            QuestionFormat::ShortAnswer
        );
    }

    #[test]
    fn review_state_exposes_proficiency_and_retrievability() {
        let now = Utc.with_ymd_and_hms(2026, 8, 7, 12, 0, 0).unwrap();
        let unassessed = ReviewState::default();
        assert_eq!(
            unassessed.proficiency_level(now),
            ProficiencyLevel::Unassessed
        );
        assert_eq!(unassessed.proficiency(now), None);
        assert_eq!(unassessed.retrievability(now), None);

        let assessed = ReviewState {
            memory: Some(MemoryState {
                stability: 7.0,
                difficulty: 4.0,
            }),
            reps: 5,
            lapses: 0,
            last_review: Some(now - Duration::days(7)),
        };
        let retrievability = assessed.retrievability(now).unwrap();
        assert!((retrievability - 0.9).abs() < 0.0001);
        let proficiency = assessed.proficiency(now).unwrap();
        assert!((proficiency - 45.0).abs() < 0.01);
        assert_eq!(assessed.proficiency_level(now), ProficiencyLevel::Medium);
    }
}
