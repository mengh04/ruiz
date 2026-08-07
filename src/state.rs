//! 全局应用状态（gpui Global）：数据库连接池 + AI 客户端 + FSRS 调度器。

use anyhow::Result;
use gpui::{App, Global};
use sqlx::SqlitePool;

use crate::ai::{client::ChatClient, image::VisionClient};
use crate::scheduler::Scheduler;

pub struct AppState {
    pub pool: SqlitePool,
    pub ai: Option<ChatClient>,
    pub vision: Option<VisionClient>,
    pub scheduler: Scheduler,
}

impl AppState {
    /// 初始化数据库并返回状态。必须在 tokio runtime 上调用（经 `gpui_tokio::Tokio::spawn`）。
    pub async fn new(db_path: &str) -> Result<Self> {
        let pool = crate::db::schema::init(db_path).await?;
        Ok(Self {
            pool,
            ai: None,
            vision: None,
            scheduler: Scheduler::new(),
        })
    }

    /// 根据本机 DeepSeek 配置刷新 AI 客户端。
    pub fn configure_ai(&mut self, config: &crate::settings::Config) {
        let c = &config.ai;
        self.ai = c
            .api_key
            .as_ref()
            .map(|key| ChatClient::new(key.clone(), c.model.clone()));
        self.vision = if c.vision.enabled() {
            VisionClient::new(
                c.vision.api_base.clone(),
                c.vision.api_key.clone().unwrap_or_default(),
                c.vision.model.clone().unwrap_or_default(),
            )
            .map_err(|error| {
                crate::diagnostics::warn(
                    "settings.vision.invalid",
                    "Vision configuration is invalid; image recognition is disabled",
                    serde_json::json!({ "error": format!("{error:#}") }),
                );
            })
            .ok()
        } else {
            None
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
