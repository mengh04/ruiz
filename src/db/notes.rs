use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::domain::note::Note;

#[allow(dead_code)] // 旧版手动建笔记 API 与数据库回归测试仍会使用
pub async fn create(pool: &SqlitePool, title: &str, content: &str) -> Result<i64> {
    let group_id = crate::db::groups::default_id(pool).await?;
    create_in_group(pool, group_id, title, content).await
}

pub async fn create_in_group(
    pool: &SqlitePool,
    group_id: i64,
    title: &str,
    content: &str,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO notes (group_id, title, content, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
    )
    .bind(group_id)
    .bind(title)
    .bind(content)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Note>> {
    let rows = sqlx::query("SELECT * FROM notes ORDER BY updated_at DESC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(from_row).collect::<Result<Vec<_>>>()
}

// 预留 CRUD API：后续功能会用到
#[allow(dead_code)]
pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Note>> {
    let row = sqlx::query("SELECT * FROM notes WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(|r| from_row(&r)).transpose()
}

#[allow(dead_code)]
pub async fn update(pool: &SqlitePool, id: i64, title: &str, content: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE notes SET title = ?1, content = ?2, updated_at = ?3 WHERE id = ?4")
        .bind(title)
        .bind(content)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn move_to_group(pool: &SqlitePool, id: i64, group_id: i64) -> Result<()> {
    let result = sqlx::query("UPDATE notes SET group_id = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(group_id)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        anyhow::bail!("要修改的笔记不存在");
    }
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    // 显式清理关联数据，避免依赖 SQLite 每条连接的 foreign_keys PRAGMA 状态。
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "DELETE FROM learning_attempts WHERE session_id IN (
             SELECT ls.id FROM learning_sessions ls JOIN learning_plans lp ON lp.id = ls.plan_id
             WHERE lp.note_id = ?1
         )",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM learning_step_progress WHERE session_id IN (
             SELECT ls.id FROM learning_sessions ls JOIN learning_plans lp ON lp.id = ls.plan_id
             WHERE lp.note_id = ?1
         )",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM learning_sessions WHERE plan_id IN (SELECT id FROM learning_plans WHERE note_id = ?1)",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM learning_prompts WHERE learning_step_id IN (
             SELECT ls.id FROM learning_steps ls JOIN learning_plans lp ON lp.id = ls.plan_id
             WHERE lp.note_id = ?1
         )",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM learning_steps WHERE plan_id IN (SELECT id FROM learning_plans WHERE note_id = ?1)",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM learning_plans WHERE note_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "DELETE FROM knowledge_unit_sources WHERE content_block_id IN (
             SELECT id FROM content_blocks WHERE note_id = ?1
         )",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM content_blocks WHERE note_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM reviews WHERE card_id IN (SELECT id FROM cards WHERE note_id = ?1)")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "DELETE FROM card_knowledge_units
         WHERE card_id IN (SELECT id FROM cards WHERE note_id = ?1)",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM cards WHERE note_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "DELETE FROM review_attempts
         WHERE knowledge_unit_id IN (SELECT id FROM knowledge_units WHERE note_id = ?1)",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM review_prompts
         WHERE knowledge_unit_id IN (SELECT id FROM knowledge_units WHERE note_id = ?1)",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM knowledge_units WHERE note_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM knowledge_claims WHERE note_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM material_analyses WHERE note_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM notes WHERE id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Note> {
    Ok(Note {
        id: row.get("id"),
        group_id: row.try_get("group_id").unwrap_or(0),
        title: row.get("title"),
        content: row.get("content"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
