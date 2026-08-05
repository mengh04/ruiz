use anyhow::Result;
use chrono::{DateTime, Utc};
use fsrs::MemoryState;
use sqlx::{Row, SqlitePool};

use crate::domain::card::Card;

#[allow(dead_code)] // 旧版手动建卡 API 与数据库回归测试仍会使用
pub async fn insert(pool: &SqlitePool, card: &Card) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO cards
            (note_id, question, standard_answer, source_excerpt,
             stability, difficulty, due, reps, lapses, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, 0, 0, ?6, ?6)
         RETURNING id",
    )
    .bind(card.note_id)
    .bind(&card.question)
    .bind(&card.standard_answer)
    .bind(&card.source_excerpt)
    .bind(card.due.to_rfc3339())
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub async fn by_note(pool: &SqlitePool, note_id: i64) -> Result<Vec<Card>> {
    let rows = sqlx::query(
        "SELECT cards.*, card_knowledge_units.knowledge_unit_id,
                knowledge_units.required_points_json
         FROM cards
         LEFT JOIN card_knowledge_units ON card_knowledge_units.card_id = cards.id
         LEFT JOIN knowledge_units ON knowledge_units.id = card_knowledge_units.knowledge_unit_id
         WHERE cards.note_id = ?1 ORDER BY cards.created_at",
    )
    .bind(note_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(from_row).collect::<Result<Vec<_>>>()
}

// 预留 CRUD API：后续功能（详情页/删除）会用到
#[allow(dead_code)]
pub async fn get(pool: &SqlitePool, id: i64) -> Result<Option<Card>> {
    let row = sqlx::query(
        "SELECT cards.*, card_knowledge_units.knowledge_unit_id,
                knowledge_units.required_points_json
         FROM cards
         LEFT JOIN card_knowledge_units ON card_knowledge_units.card_id = cards.id
         LEFT JOIN knowledge_units ON knowledge_units.id = card_knowledge_units.knowledge_unit_id
         WHERE cards.id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| from_row(&r)).transpose()
}

/// 到期（due <= now）的卡片，按到期时间升序 —— 复习队列。
#[allow(dead_code)]
pub async fn due(pool: &SqlitePool, now: DateTime<Utc>) -> Result<Vec<Card>> {
    due_in_scope(pool, now, None, None).await
}

/// 查询指定分组或章节的到期卡片。分组筛选通过笔记章节归属实现，
/// 因此“整个分组”和“单篇章节”始终复用同一套卡片与记忆状态。
pub async fn due_in_scope(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    group_id: Option<i64>,
    note_id: Option<i64>,
) -> Result<Vec<Card>> {
    let rows = sqlx::query(
        "SELECT cards.*, card_knowledge_units.knowledge_unit_id,
                knowledge_units.required_points_json
         FROM cards
         LEFT JOIN card_knowledge_units ON card_knowledge_units.card_id = cards.id
         LEFT JOIN knowledge_units ON knowledge_units.id = card_knowledge_units.knowledge_unit_id
         INNER JOIN notes ON notes.id = cards.note_id
         WHERE cards.due <= ?1
           AND (?2 IS NULL OR notes.group_id = ?2)
           AND (?3 IS NULL OR cards.note_id = ?3)
         ORDER BY cards.due ASC",
    )
    .bind(now.to_rfc3339())
    .bind(group_id)
    .bind(note_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(from_row).collect::<Result<Vec<_>>>()
}

/// 复习后更新 FSRS 记忆状态与调度信息。
pub async fn update_schedule(
    pool: &SqlitePool,
    id: i64,
    memory: MemoryState,
    due: DateTime<Utc>,
    reps: u32,
    lapses: u32,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE cards
         SET stability = ?1, difficulty = ?2, due = ?3, reps = ?4, lapses = ?5,
             last_review = ?6, updated_at = ?6
         WHERE id = ?7",
    )
    .bind(memory.stability)
    .bind(memory.difficulty)
    .bind(due.to_rfc3339())
    .bind(reps as i64)
    .bind(lapses as i64)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM card_knowledge_units WHERE card_id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM cards WHERE id = ?1")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Card> {
    let stability: Option<f32> = row.get("stability");
    let difficulty: Option<f32> = row.get("difficulty");
    let memory = match (stability, difficulty) {
        (Some(s), Some(d)) => Some(MemoryState {
            stability: s,
            difficulty: d,
        }),
        _ => None,
    };
    Ok(Card {
        id: row.get("id"),
        note_id: row.get("note_id"),
        question: row.get("question"),
        standard_answer: row.get("standard_answer"),
        source_excerpt: row.get("source_excerpt"),
        knowledge_unit_id: row
            .try_get::<Option<i64>, _>("knowledge_unit_id")
            .unwrap_or(None),
        required_points: row
            .try_get::<String, _>("required_points_json")
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default(),
        memory,
        due: row.get::<DateTime<Utc>, _>("due"),
        reps: row.get::<i64, _>("reps") as u32,
        lapses: row.get::<i64, _>("lapses") as u32,
        last_review: row.get("last_review"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
