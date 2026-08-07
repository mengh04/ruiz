//! AI 层：OpenAI 兼容 Chat Completions 客户端 + 出题 + 判官。

pub mod client;
pub mod dynamic_question;
pub mod generate;
pub mod image;
pub mod import;
pub mod judge;
pub mod learning_plan;
pub mod learning_question;
pub mod plan;
pub mod progress;
pub mod prompts;
pub mod source;
pub(crate) mod text;
pub mod workflow;
