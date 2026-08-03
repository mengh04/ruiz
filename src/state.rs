//! 全局应用状态（gpui Global）：数据库连接池 + AI 客户端 + FSRS 调度器。

use anyhow::Result;
use gpui::{App, Global};
use sqlx::SqlitePool;

use crate::ai::client::ChatClient;
use crate::scheduler::Scheduler;

pub struct AppState {
    pub pool: SqlitePool,
    pub ai: Option<ChatClient>,
    pub scheduler: Scheduler,
}

impl AppState {
    /// 初始化数据库并返回状态。必须在 tokio runtime 上调用（经 `gpui_tokio::Tokio::spawn`）。
    pub async fn new(db_path: &str) -> Result<Self> {
        let pool = crate::db::schema::init(db_path).await?;
        Ok(Self {
            pool,
            ai: None,
            scheduler: Scheduler::new(),
        })
    }

    /// 根据配置刷新 AI 客户端（三个字段齐全才启用）。
    pub fn configure_ai(&mut self, config: &crate::settings::Config) {
        let c = &config.ai;
        self.ai = match (&c.api_base, &c.api_key, &c.model) {
            (Some(base), Some(key), Some(model)) => {
                Some(ChatClient::new(base.clone(), key.clone(), model.clone()))
            }
            _ => None,
        };
    }

    pub fn global(cx: &App) -> &AppState {
        cx.global::<AppState>()
    }

    pub fn global_mut(cx: &mut App) -> &mut AppState {
        cx.global_mut::<AppState>()
    }
}

impl Global for AppState {}
