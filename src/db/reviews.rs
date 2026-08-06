use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::domain::review::{Rating, Review};

// 预留 CRUD API：后续功能（卡片历史）会用到
#[allow(dead_code)]
pub async fn by_card(pool: &SqlitePool, card_id: i64) -> Result<Vec<Review>> {
    let rows = sqlx::query("SELECT * FROM reviews WHERE card_id = ?1 ORDER BY reviewed_at DESC")
        .bind(card_id)
        .fetch_all(pool)
        .await?;
    rows.iter().map(from_row).collect::<Result<Vec<_>>>()
}

// 预留 CRUD API：后续功能（卡片历史）会用到
#[allow(dead_code)]
fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Review> {
    Ok(Review {
        id: row.get("id"),
        card_id: row.get("card_id"),
        user_answer: row.get("user_answer"),
        ai_feedback: row.get("ai_feedback"),
        rating: Rating::from_fsrs(row.get::<i64, _>("rating") as u32)
            .expect("数据库里的 rating 非法"),
        reviewed_at: row.get::<DateTime<Utc>, _>("reviewed_at"),
    })
}
