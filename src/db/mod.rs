//! 持久化层：sqlx + SQLite。
//! 所有查询手写 SQL（不用编译期宏，避免 DATABASE_URL 的 offline 复杂度），
//! 时间统一存 RFC3339 TEXT（sqlx 的 chrono 特性负责映射）。

pub mod cards;
pub mod groups;
pub mod knowledge;
pub mod notes;
pub mod reviews;
pub mod schema;
