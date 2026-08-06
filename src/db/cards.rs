use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::domain::card::Card;

#[allow(dead_code)] // 旧版手动建卡 API 与数据库回归测试仍会使用
pub async fn insert(pool: &SqlitePool, card: &Card) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let mut transaction = pool.begin().await?;
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
    .fetch_one(&mut *transaction)
    .await?;
    let card_id: i64 = row.get("id");
    let evidence = serde_json::to_string(&vec![
        card.source_excerpt
            .clone()
            .unwrap_or_else(|| card.standard_answer.clone()),
    ])?;
    let required_points = serde_json::to_string(&vec![card.standard_answer.clone()])?;
    let unit_id: i64 = sqlx::query(
        "INSERT INTO knowledge_units
            (note_id, local_id, topic, objective, unit_type, importance, stage,
             cognitive_action, required_points_json, claim_ids_json, evidence_json,
             reason, quick, recommended, generated, stability, difficulty, due,
             reps, lapses, last_review, prerequisite_ids_json, position)
         VALUES (?1, ?2, '旧版卡片', ?3, 'legacy', 'core', 'foundation',
                 'recall', ?4, '[]', ?5, '由旧版卡片迁移', 1, 1, 1,
                 NULL, NULL, ?6, 0, 0, NULL, '[]', ?7)
         RETURNING id",
    )
    .bind(card.note_id)
    .bind(format!("legacy-card-{card_id}"))
    .bind(&card.question)
    .bind(required_points)
    .bind(evidence)
    .bind(card.due.to_rfc3339())
    .bind(card_id)
    .fetch_one(&mut *transaction)
    .await?
    .get("id");
    sqlx::query("INSERT INTO card_knowledge_units (card_id, knowledge_unit_id) VALUES (?1, ?2)")
        .bind(card_id)
        .bind(unit_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(card_id)
}

#[allow(dead_code)] // 兼容旧版卡片详情查询，当前界面以知识单元为主。
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

// 预留 CRUD API：后续功能（详情页/删除）会用到
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
    Ok(Card {
        note_id: row.get("note_id"),
        question: row.get("question"),
        standard_answer: row.get("standard_answer"),
        source_excerpt: row.get("source_excerpt"),
        knowledge_unit_id: row
            .try_get::<Option<i64>, _>("knowledge_unit_id")
            .unwrap_or(None),
        due: row.get::<DateTime<Utc>, _>("due"),
        reps: row.get::<i64, _>("reps") as u32,
        lapses: row.get::<i64, _>("lapses") as u32,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}
