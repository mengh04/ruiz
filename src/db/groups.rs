use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::domain::group::{GroupSummary, StudyGroup};

const DEFAULT_GROUP: &str = "未分组";

pub async fn list(pool: &SqlitePool) -> Result<Vec<StudyGroup>> {
    let rows = sqlx::query("SELECT * FROM study_groups ORDER BY name COLLATE NOCASE")
        .fetch_all(pool)
        .await?;
    rows.iter().map(from_row).collect()
}

pub async fn summaries(pool: &SqlitePool, now: DateTime<Utc>) -> Result<Vec<GroupSummary>> {
    let rows = sqlx::query(
        "SELECT g.*, COUNT(DISTINCT n.id) AS note_count,
                COUNT(DISTINCT c.id) AS card_count,
                COUNT(DISTINCT CASE WHEN c.due <= ?1 THEN c.id END) AS due_count
         FROM study_groups g
         LEFT JOIN notes n ON n.group_id = g.id
         LEFT JOIN cards c ON c.note_id = n.id
         GROUP BY g.id ORDER BY g.name COLLATE NOCASE",
    )
    .bind(now.to_rfc3339())
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(GroupSummary {
                group: from_row(row)?,
                note_count: row.get::<i64, _>("note_count") as usize,
                card_count: row.get::<i64, _>("card_count") as usize,
                due_count: row.get::<i64, _>("due_count") as usize,
            })
        })
        .collect()
}

pub async fn get_or_create(pool: &SqlitePool, name: &str) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("分组名称不能为空"));
    }
    sqlx::query("INSERT INTO study_groups (name, created_at, updated_at) VALUES (?1, ?2, ?2) ON CONFLICT(name) DO UPDATE SET updated_at = excluded.updated_at")
        .bind(name)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    Ok(
        sqlx::query("SELECT id FROM study_groups WHERE name = ?1 COLLATE NOCASE")
            .bind(name)
            .fetch_one(pool)
            .await?
            .get("id"),
    )
}

pub async fn default_id(pool: &SqlitePool) -> Result<i64> {
    get_or_create(pool, DEFAULT_GROUP).await
}

pub async fn rename(pool: &SqlitePool, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("分组名称不能为空"));
    }
    let existing = sqlx::query("SELECT id FROM study_groups WHERE name = ?1 COLLATE NOCASE")
        .bind(name)
        .fetch_optional(pool)
        .await?
        .map(|row| row.get::<i64, _>("id"));
    if existing.is_some_and(|existing_id| existing_id != id) {
        return Err(anyhow!("已经存在名为“{name}”的分组"));
    }
    let result = sqlx::query("UPDATE study_groups SET name = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(name)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow!("要修改的分组不存在"));
    }
    Ok(())
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<StudyGroup> {
    Ok(StudyGroup {
        id: row.get("id"),
        name: row.get("name"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
