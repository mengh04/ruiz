//! 持久化层：sqlx + SQLite。
//! 查询手写 SQL（不用 `query!` 编译期宏，避免 DATABASE_URL 的 offline 复杂度）；
//! 数据库升级统一用 `sqlx::migrate!`（编译期嵌入 `migrations/`，按版本只执行一次）。
//! 时间统一存 RFC3339 TEXT（sqlx 的 chrono 特性负责映射）。

pub mod dynamic_reviews;
pub mod groups;
pub mod knowledge;
pub mod learning;
pub mod notes;
pub mod schema;
