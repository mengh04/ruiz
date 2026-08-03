use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 用户导入的学习材料：一段网页复制文本 / 一个 markdown 文件等。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    /// 用户给材料起的名字，例如「计算机网络 第三章 数据链路层」
    pub title: String,
    /// 原文全文
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
