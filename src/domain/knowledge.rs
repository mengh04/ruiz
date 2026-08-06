use serde::{Deserialize, Serialize};

use super::dynamic_review::ReviewState;

/// 材料分析摘要，用于解释 AI 为什么推荐当前题量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialAnalysis {
    pub note_id: i64,
    pub summary: String,
    pub document_type: String,
    pub warnings: Vec<String>,
    pub quick_count: usize,
    pub recommended_count: usize,
    pub comprehensive_count: usize,
}

/// 材料中最小的、可追踪到证据的事实陈述。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct KnowledgeClaim {
    pub local_id: String,
    pub statement: String,
    pub importance: String,
    pub evidence: Vec<String>,
}

/// 一个可独立复习、独立判分的学习目标。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeUnit {
    pub id: i64,
    pub note_id: i64,
    pub local_id: String,
    pub topic: String,
    pub objective: String,
    pub unit_type: String,
    pub importance: String,
    pub stage: String,
    pub cognitive_action: String,
    pub required_points: Vec<String>,
    pub claim_ids: Vec<String>,
    pub evidence: Vec<String>,
    pub reason: String,
    pub quick: bool,
    pub recommended: bool,
    pub generated: bool,
    #[serde(skip)]
    pub review_state: ReviewState,
    pub prerequisite_unit_ids: Vec<String>,
    pub position: usize,
}
