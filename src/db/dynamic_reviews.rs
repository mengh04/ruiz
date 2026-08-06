use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use fsrs::MemoryState;
use sqlx::{Row, SqlitePool};

use crate::domain::{
    dynamic_review::{ReviewItem, ReviewPrompt},
    review::Rating,
};

pub async fn due_in_scope(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    group_ids: &[i64],
    note_id: Option<i64>,
) -> Result<Vec<ReviewItem>> {
    let group_ids_json = serde_json::to_string(group_ids)?;
    let rows = sqlx::query(
        "SELECT ku.*, n.title AS note_title,
                c.id AS seed_card_id, c.question AS fallback_question,
                c.standard_answer AS fallback_answer,
                c.source_excerpt AS fallback_source
         FROM knowledge_units ku
         JOIN notes n ON n.id = ku.note_id
         LEFT JOIN cards c ON c.id = (
             SELECT cku.card_id FROM card_knowledge_units cku
             WHERE cku.knowledge_unit_id = ku.id
             ORDER BY cku.card_id LIMIT 1
         )
         WHERE ku.generated = 1 AND ku.introduced_at IS NOT NULL AND ku.due <= ?1
           AND (json_array_length(?2) = 0 OR n.group_id IN (
               SELECT value FROM json_each(?2)
           ))
           AND (?3 IS NULL OR ku.note_id = ?3)
         ORDER BY ku.due ASC, ku.id ASC",
    )
    .bind(now.to_rfc3339())
    .bind(group_ids_json)
    .bind(note_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn recent_questions(
    pool: &SqlitePool,
    unit_id: i64,
    limit: usize,
) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT question FROM review_prompts
         WHERE knowledge_unit_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT ?2",
    )
    .bind(unit_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(|row| row.get("question")).collect())
}

pub async fn insert_prompt(pool: &SqlitePool, prompt: &ReviewPrompt) -> Result<i64> {
    let row = sqlx::query(
        "INSERT INTO review_prompts
            (knowledge_unit_id, question_type, mastery_band, question, options_json,
             standard_answer, required_points_json, source_excerpt, generation_mode, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         RETURNING id",
    )
    .bind(prompt.unit_id)
    .bind(prompt.format.as_str())
    .bind(prompt.mastery.as_str())
    .bind(&prompt.question)
    .bind(serde_json::to_string(&prompt.options)?)
    .bind(&prompt.standard_answer)
    .bind(serde_json::to_string(&prompt.required_points)?)
    .bind(&prompt.source_excerpt)
    .bind(&prompt.generation_mode)
    .bind(Utc::now().to_rfc3339())
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

#[allow(clippy::too_many_arguments)]
pub async fn complete_review(
    pool: &SqlitePool,
    item: &ReviewItem,
    prompt_id: Option<i64>,
    user_answer: &str,
    ai_feedback: &str,
    rating: Rating,
    memory: MemoryState,
    due: DateTime<Utc>,
    reps: u32,
    lapses: u32,
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE knowledge_units
         SET stability = ?1, difficulty = ?2, due = ?3, reps = ?4,
             lapses = ?5, last_review = ?6
         WHERE id = ?7",
    )
    .bind(memory.stability)
    .bind(memory.difficulty)
    .bind(due.to_rfc3339())
    .bind(reps as i64)
    .bind(lapses as i64)
    .bind(&now)
    .bind(item.unit_id)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        return Err(anyhow!("要更新的知识单元不存在"));
    }
    sqlx::query(
        "INSERT INTO review_attempts
            (knowledge_unit_id, prompt_id, seed_card_id, user_answer,
             ai_feedback, rating, reviewed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(item.unit_id)
    .bind(prompt_id)
    .bind(item.seed_card_id)
    .bind(user_answer)
    .bind(ai_feedback)
    .bind(rating as i64)
    .bind(&now)
    .execute(&mut *transaction)
    .await?;

    if let Some(card_id) = item.seed_card_id {
        sqlx::query(
            "UPDATE cards
             SET stability = ?1, difficulty = ?2, due = ?3, reps = ?4,
                 lapses = ?5, last_review = ?6, updated_at = ?6
             WHERE id = ?7",
        )
        .bind(memory.stability)
        .bind(memory.difficulty)
        .bind(due.to_rfc3339())
        .bind(reps as i64)
        .bind(lapses as i64)
        .bind(&now)
        .bind(card_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO reviews (card_id, user_answer, ai_feedback, rating, reviewed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(card_id)
        .bind(user_answer)
        .bind(ai_feedback)
        .bind(rating as i64)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ReviewItem> {
    let stability: Option<f32> = row.get("stability");
    let difficulty: Option<f32> = row.get("difficulty");
    let memory = match (stability, difficulty) {
        (Some(stability), Some(difficulty)) => Some(MemoryState {
            stability,
            difficulty,
        }),
        _ => None,
    };
    Ok(ReviewItem {
        unit_id: row.get("id"),
        note_title: row.get("note_title"),
        topic: row.get("topic"),
        objective: row.get("objective"),
        unit_type: row.get("unit_type"),
        cognitive_action: row.get("cognitive_action"),
        required_points: parse_json(row.get("required_points_json"))?,
        evidence: parse_json(row.get("evidence_json"))?,
        seed_card_id: row.get("seed_card_id"),
        fallback_question: row.get("fallback_question"),
        fallback_answer: row.get("fallback_answer"),
        fallback_source: row.get("fallback_source"),
        memory,
        reps: row.get::<i64, _>("reps") as u32,
        lapses: row.get::<i64, _>("lapses") as u32,
        last_review: row.get("last_review"),
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(value: String) -> Result<T> {
    serde_json::from_str(&value).map_err(|error| anyhow!("知识单元 JSON 数据损坏: {error}"))
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use fsrs::MemoryState;
    use sqlx::Row;

    use super::*;
    use crate::{db, domain::card::Card};

    #[tokio::test]
    async fn due_scope_combines_multiple_groups() {
        let pool = db::schema::init("sqlite::memory:")
            .await
            .expect("内存库建表失败");
        let rust_group = db::groups::get_or_create(&pool, "Rust").await.unwrap();
        let network_group = db::groups::get_or_create(&pool, "网络").await.unwrap();
        let database_group = db::groups::get_or_create(&pool, "数据库").await.unwrap();

        for (group_id, title) in [
            (rust_group, "所有权"),
            (network_group, "TCP"),
            (database_group, "事务"),
        ] {
            let note_id = db::notes::create_in_group(&pool, group_id, title, title)
                .await
                .unwrap();
            db::cards::insert(
                &pool,
                &Card::new(
                    note_id,
                    format!("{title}问题"),
                    format!("{title}答案"),
                    None,
                ),
            )
            .await
            .unwrap();
        }

        let filtered = due_in_scope(&pool, Utc::now(), &[rust_group, database_group], None)
            .await
            .unwrap();
        let mut titles = filtered
            .into_iter()
            .map(|item| item.note_title)
            .collect::<Vec<_>>();
        titles.sort();

        assert_eq!(titles, vec!["事务", "所有权"]);
    }

    #[tokio::test]
    async fn prompt_snapshot_and_review_completion_are_atomic() {
        let pool = db::schema::init("sqlite::memory:")
            .await
            .expect("内存库建表失败");
        let note_id = db::notes::create(&pool, "Rust 所有权", "所有权规则与借用")
            .await
            .unwrap();
        let card_id = db::cards::insert(
            &pool,
            &Card::new(
                note_id,
                "所有权转移后原变量还能使用吗？".into(),
                "不能，除非类型实现 Copy。".into(),
                Some("赋值会移动所有权。".into()),
            ),
        )
        .await
        .unwrap();
        let item = due_in_scope(&pool, Utc::now(), &[], None)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let mut prompt = item.fallback_prompt();
        prompt.format = crate::domain::dynamic_review::QuestionFormat::Choice;
        prompt.options = vec!["可以".into(), "不可以".into(), "只在函数中可以".into()];
        prompt.standard_answer = "不可以".into();
        prompt.generation_mode = "ai".into();
        let prompt_id = insert_prompt(&pool, &prompt).await.unwrap();

        assert_eq!(
            recent_questions(&pool, item.unit_id, 5).await.unwrap(),
            vec![prompt.question.clone()]
        );
        let stored_prompt = sqlx::query(
            "SELECT question_type, options_json, standard_answer, generation_mode
             FROM review_prompts WHERE id = ?1",
        )
        .bind(prompt_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored_prompt.get::<String, _>("question_type"), "choice");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&stored_prompt.get::<String, _>("options_json"))
                .unwrap(),
            prompt.options
        );
        assert_eq!(stored_prompt.get::<String, _>("standard_answer"), "不可以");
        assert_eq!(stored_prompt.get::<String, _>("generation_mode"), "ai");

        let memory = MemoryState {
            stability: 3.5,
            difficulty: 5.25,
        };
        let due = Utc::now() + Duration::days(4);
        complete_review(
            &pool,
            &item,
            Some(prompt_id),
            "不可以",
            "回答正确",
            Rating::Good,
            memory,
            due,
            1,
            0,
        )
        .await
        .unwrap();

        let unit = sqlx::query(
            "SELECT stability, difficulty, due, reps, lapses
             FROM knowledge_units WHERE id = ?1",
        )
        .bind(item.unit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unit.get::<i64, _>("reps"), 1);
        assert_eq!(unit.get::<i64, _>("lapses"), 0);
        assert!((unit.get::<f64, _>("stability") - 3.5).abs() < f64::EPSILON);
        assert!((unit.get::<f64, _>("difficulty") - 5.25).abs() < f64::EPSILON);
        assert_eq!(unit.get::<String, _>("due"), due.to_rfc3339());

        let attempt = sqlx::query(
            "SELECT prompt_id, seed_card_id, user_answer, ai_feedback, rating
             FROM review_attempts WHERE knowledge_unit_id = ?1",
        )
        .bind(item.unit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(attempt.get::<i64, _>("prompt_id"), prompt_id);
        assert_eq!(attempt.get::<i64, _>("seed_card_id"), card_id);
        assert_eq!(attempt.get::<String, _>("user_answer"), "不可以");
        assert_eq!(attempt.get::<String, _>("ai_feedback"), "回答正确");
        assert_eq!(attempt.get::<i64, _>("rating"), Rating::Good as i64);

        let card = db::cards::get(&pool, card_id).await.unwrap().unwrap();
        assert_eq!(card.reps, 1);
        assert_eq!(card.due, due);
        assert_eq!(db::reviews::by_card(&pool, card_id).await.unwrap().len(), 1);

        sqlx::raw_sql(
            "CREATE TRIGGER reject_forced_attempt
             BEFORE INSERT ON review_attempts
             WHEN NEW.user_answer = 'force rollback'
             BEGIN
                 SELECT RAISE(ABORT, 'forced rollback');
             END;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let failed = complete_review(
            &pool,
            &item,
            Some(prompt_id),
            "force rollback",
            "不应保存",
            Rating::Again,
            MemoryState {
                stability: 0.5,
                difficulty: 9.0,
            },
            Utc::now() + Duration::hours(1),
            2,
            1,
        )
        .await;
        assert!(failed.is_err());

        let unit_after_rollback = sqlx::query(
            "SELECT stability, difficulty, due, reps, lapses
             FROM knowledge_units WHERE id = ?1",
        )
        .bind(item.unit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unit_after_rollback.get::<i64, _>("reps"), 1);
        assert_eq!(unit_after_rollback.get::<i64, _>("lapses"), 0);
        assert_eq!(
            unit_after_rollback.get::<String, _>("due"),
            due.to_rfc3339()
        );
        let attempt_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count FROM review_attempts WHERE knowledge_unit_id = ?1",
        )
        .bind(item.unit_id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("count");
        assert_eq!(attempt_count, 1);
    }
}
