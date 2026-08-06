//! AI 层：OpenAI 兼容 Chat Completions 客户端 + 出题 + 判官。

pub mod client;
pub mod dynamic_question;
pub mod generate;
pub mod import;
pub mod judge;
pub mod plan;
pub mod progress;
pub mod prompts;
pub(crate) mod text;
pub mod workflow;
