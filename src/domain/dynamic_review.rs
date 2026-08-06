use chrono::{DateTime, Utc};
use fsrs::MemoryState;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasteryBand {
    Beginner,
    Developing,
    Strong,
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
}
