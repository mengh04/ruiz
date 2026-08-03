use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::domain::note::Note;

pub async fn create(pool: &SqlitePool, title: &str, content: &str) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO notes (title, content, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3) RETURNING id",
    )
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

#[allow(dead_code)]
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM notes WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Note> {
    Ok(Note {
        id: row.get("id"),
        title: row.get("title"),
        content: row.get("content"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
