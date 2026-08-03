//! Ruiz 的模型提示词。
//!
//! 长提示词独立为 Markdown，便于评审、版本管理和后续建立快照测试。

pub const GENERATE_SYSTEM: &str = include_str!("generate_system.md");
pub const JUDGE_SYSTEM: &str = include_str!("judge_system.md");
