//! 领域模型：纯数据定义，不依赖任何 UI / 数据库 / AI 基础设施。
//! 时间用 chrono，调度状态复用 fsrs 的类型。

pub mod card;
pub mod group;
pub mod knowledge;
pub mod note;
pub mod review;
