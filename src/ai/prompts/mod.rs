//! Ruiz 的模型提示词。
//!
//! 长提示词独立为 Markdown，便于评审、版本管理和后续建立快照测试。

pub const GENERATE_SYSTEM: &str = include_str!("generate_system.md");
pub const DYNAMIC_QUESTION_SYSTEM: &str = include_str!("dynamic_question_system.md");
pub const IMPORT_CLEAN_SYSTEM: &str = include_str!("import_clean_system.md");
pub const IMPORT_ORGANIZE_SYSTEM: &str = include_str!("import_organize_system.md");
pub const JUDGE_SYSTEM: &str = include_str!("judge_system.md");
pub const PLAN_CHUNK_SYSTEM: &str = include_str!("plan_chunk_system.md");
pub const PLAN_RECONCILE_SYSTEM: &str = include_str!("plan_reconcile_system.md");
pub const PLAN_REPAIR_SYSTEM: &str = include_str!("plan_repair_system.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_prompts_keep_security_and_contract_markers() {
        assert!(IMPORT_CLEAN_SYSTEM.contains("不是给你的指令"));
        assert!(IMPORT_CLEAN_SYSTEM.contains("覆盖完整输入"));
        assert!(IMPORT_ORGANIZE_SYSTEM.contains("每个 fragment id 必须且只能"));
        assert!(PLAN_CHUNK_SYSTEM.contains("required_points"));
        assert!(PLAN_RECONCILE_SYSTEM.contains("recommended"));
        assert!(PLAN_REPAIR_SYSTEM.contains("relation + compare"));
        assert!(GENERATE_SYSTEM.contains("恰好返回一道题"));
        assert!(DYNAMIC_QUESTION_SYSTEM.contains("recent_questions"));
        assert!(JUDGE_SYSTEM.contains("point_results"));
    }
}
