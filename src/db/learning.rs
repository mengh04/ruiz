use std::collections::HashMap;

use anyhow::{Result, anyhow};
use chrono::{Duration, Utc};
use sqlx::{Row, SqlitePool};

use crate::domain::{
    dynamic_review::QuestionFormat,
    knowledge::KnowledgeUnit,
    learning::{
        ContentBlock, ContentBlockKind, LearningIntent, LearningPlan, LearningPrompt,
        LearningSession, LearningStep, LearningStepKind, UnitSourceLink, fallback_plan,
        map_unit_sources, parse_content_blocks, validate_plan,
    },
    note::Note,
};

#[allow(dead_code)]
pub async fn prepare_fallback_plan(
    pool: &SqlitePool,
    note: &Note,
    units: &[KnowledgeUnit],
) -> Result<LearningPlan> {
    let hash = crate::domain::learning::content_hash(&note.content);
    if let Some(plan) = latest_valid_plan(pool, note.id, &hash).await? {
        return Ok(plan);
    }
    let blocks = parse_content_blocks(note.id, &note.content);
    if blocks.is_empty() {
        return Err(anyhow!("笔记正文为空，无法开始学习"));
    }
    let links = map_unit_sources(units, &blocks);
    let plan = fallback_plan(
        note.id,
        &note.title,
        &blocks[0].content_hash,
        &blocks,
        units,
        &links,
    );
    validate_plan(&plan, &blocks, units)?;
    save_plan(pool, &blocks, &links, &plan).await?;
    latest_valid_plan(pool, note.id, &blocks[0].content_hash)
        .await?
        .ok_or_else(|| anyhow!("学习路线保存后无法读取"))
}

pub async fn latest_valid_plan(
    pool: &SqlitePool,
    note_id: i64,
    content_hash: &str,
) -> Result<Option<LearningPlan>> {
    let row = sqlx::query(
        "SELECT * FROM learning_plans
         WHERE note_id = ?1 AND content_hash = ?2
         ORDER BY plan_version DESC, id DESC LIMIT 1",
    )
    .bind(note_id)
    .bind(content_hash)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let plan_id: i64 = row.get("id");
    let step_rows =
        sqlx::query("SELECT * FROM learning_steps WHERE plan_id = ?1 ORDER BY position, id")
            .bind(plan_id)
            .fetch_all(pool)
            .await?;
    let steps = step_rows
        .iter()
        .map(step_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(LearningPlan {
        id: Some(plan_id),
        note_id: row.get("note_id"),
        content_hash: row.get("content_hash"),
        plan_version: row.get("plan_version"),
        summary: row.get("summary"),
        estimated_minutes: row.get::<i64, _>("estimated_minutes") as usize,
        generation_mode: row.get("generation_mode"),
        topics: parse_json(row.get("topics_json"), "topics_json")?,
        steps,
    }))
}

pub async fn blocks_for_plan(pool: &SqlitePool, plan: &LearningPlan) -> Result<Vec<ContentBlock>> {
    let rows = sqlx::query(
        "SELECT * FROM content_blocks
         WHERE note_id = ?1 AND content_hash = ?2 ORDER BY position",
    )
    .bind(plan.note_id)
    .bind(&plan.content_hash)
    .fetch_all(pool)
    .await?;
    rows.iter().map(block_from_row).collect()
}

pub async fn save_plan(
    pool: &SqlitePool,
    blocks: &[ContentBlock],
    links: &[UnitSourceLink],
    plan: &LearningPlan,
) -> Result<i64> {
    let mut transaction = pool.begin().await?;
    for block in blocks {
        sqlx::query(
            "INSERT INTO content_blocks
                (note_id, content_hash, local_id, kind, heading_path_json, source_start,
                 source_end, source_text, plain_text, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(note_id, content_hash, local_id) DO UPDATE SET
                 kind = excluded.kind, heading_path_json = excluded.heading_path_json,
                 source_start = excluded.source_start, source_end = excluded.source_end,
                 source_text = excluded.source_text, plain_text = excluded.plain_text,
                 position = excluded.position",
        )
        .bind(block.note_id)
        .bind(&block.content_hash)
        .bind(&block.local_id)
        .bind(block.kind.as_str())
        .bind(serde_json::to_string(&block.heading_path)?)
        .bind(block.source_start as i64)
        .bind(block.source_end as i64)
        .bind(&block.source_text)
        .bind(&block.plain_text)
        .bind(block.position as i64)
        .execute(&mut *transaction)
        .await?;
    }
    for link in links {
        sqlx::query(
            "INSERT INTO knowledge_unit_sources
                (knowledge_unit_id, content_block_id, relevance, position)
             SELECT ?1, id, ?4, ?5 FROM content_blocks
             WHERE note_id = ?2 AND content_hash = ?3 AND local_id = ?6
             ON CONFLICT(knowledge_unit_id, content_block_id) DO UPDATE SET
                 relevance = excluded.relevance, position = excluded.position",
        )
        .bind(link.unit_id)
        .bind(plan.note_id)
        .bind(&plan.content_hash)
        .bind(link.relevance.as_str())
        .bind(link.position as i64)
        .bind(&link.block_local_id)
        .execute(&mut *transaction)
        .await?;
    }
    let now = Utc::now().to_rfc3339();
    let plan_id: i64 = sqlx::query(
        "INSERT INTO learning_plans
            (note_id, content_hash, plan_version, summary, estimated_minutes,
             generation_mode, topics_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(note_id, content_hash, plan_version) DO UPDATE SET
             summary = excluded.summary, estimated_minutes = excluded.estimated_minutes,
             generation_mode = excluded.generation_mode, topics_json = excluded.topics_json
         RETURNING id",
    )
    .bind(plan.note_id)
    .bind(&plan.content_hash)
    .bind(plan.plan_version)
    .bind(&plan.summary)
    .bind(plan.estimated_minutes as i64)
    .bind(&plan.generation_mode)
    .bind(serde_json::to_string(&plan.topics)?)
    .bind(&now)
    .fetch_one(&mut *transaction)
    .await?
    .get("id");
    let active: i64 = sqlx::query(
        "SELECT COUNT(*) AS count FROM learning_sessions
         WHERE plan_id = ?1 AND status IN ('not_started', 'active', 'paused')",
    )
    .bind(plan_id)
    .fetch_one(&mut *transaction)
    .await?
    .get("count");
    if active == 0 {
        sqlx::query("DELETE FROM learning_steps WHERE plan_id = ?1")
            .bind(plan_id)
            .execute(&mut *transaction)
            .await?;
        for step in &plan.steps {
            sqlx::query(
                "INSERT INTO learning_steps
                    (plan_id, local_id, topic_id, topic_title, kind, block_ids_json,
                     unit_ids_json, source_step_ids_json, intent, question_format, reason, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .bind(plan_id)
            .bind(&step.local_id)
            .bind(&step.topic_id)
            .bind(&step.topic_title)
            .bind(step.kind.as_str())
            .bind(serde_json::to_string(&step.block_ids)?)
            .bind(serde_json::to_string(&step.unit_ids)?)
            .bind(serde_json::to_string(&step.source_step_ids)?)
            .bind(step.intent.map(LearningIntent::as_str))
            .bind(step.question_format.map(QuestionFormat::as_str))
            .bind(&step.reason)
            .bind(step.position as i64)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(plan_id)
}

pub async fn resume_or_start_session(pool: &SqlitePool, plan_id: i64) -> Result<LearningSession> {
    if let Some(row) = sqlx::query(
        "SELECT * FROM learning_sessions
         WHERE plan_id = ?1 AND status IN ('not_started', 'active', 'paused')
         ORDER BY id DESC LIMIT 1",
    )
    .bind(plan_id)
    .fetch_optional(pool)
    .await?
    {
        let id: i64 = row.get("id");
        sqlx::query(
            "UPDATE learning_sessions SET status = 'active', updated_at = ?2 WHERE id = ?1",
        )
        .bind(id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        return Ok(LearningSession {
            id,
            plan_id,
            current_step_index: row.get::<i64, _>("current_step_index") as usize,
        });
    }
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO learning_sessions
            (plan_id, status, current_step_index, started_at, updated_at)
         VALUES (?1, 'active', 0, ?2, ?2) RETURNING id",
    )
    .bind(plan_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(LearningSession {
        id: row.get("id"),
        plan_id,
        current_step_index: 0,
    })
}

pub async fn pause_session(pool: &SqlitePool, session_id: i64) -> Result<()> {
    sqlx::query("UPDATE learning_sessions SET status = 'paused', updated_at = ?2 WHERE id = ?1")
        .bind(session_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn learning_progress_state(pool: &SqlitePool, note_id: i64) -> Result<(bool, bool)> {
    let row = sqlx::query(
        "SELECT ls.status
         FROM learning_sessions ls
         JOIN learning_plans lp ON lp.id = ls.plan_id
         WHERE lp.note_id = ?1
         ORDER BY ls.id DESC LIMIT 1",
    )
    .bind(note_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok((false, false));
    };
    let status: String = row.get("status");
    Ok((
        true,
        matches!(status.as_str(), "not_started" | "active" | "paused"),
    ))
}

pub async fn reset_learning_progress(pool: &SqlitePool, note_id: i64) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM learning_sessions
         WHERE plan_id IN (SELECT id FROM learning_plans WHERE note_id = ?1)",
    )
    .bind(note_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn recap_candidates(
    pool: &SqlitePool,
    session_id: i64,
    topic_unit_ids: &[String],
) -> Result<Vec<String>> {
    let mut candidates = Vec::new();
    let attempt_rows = sqlx::query(
        "SELECT unit_ids_json FROM learning_attempts
         WHERE session_id = ?1 AND attempt_number = 1 AND result IN ('partial', 'incorrect')
         ORDER BY CASE result WHEN 'incorrect' THEN 0 ELSE 1 END, id",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    for row in attempt_rows {
        let unit_ids: Vec<String> = parse_json(row.get("unit_ids_json"), "unit_ids_json")?;
        for unit_id in unit_ids {
            if topic_unit_ids.contains(&unit_id) && !candidates.contains(&unit_id) {
                candidates.push(unit_id);
            }
        }
    }
    Ok(candidates)
}

pub async fn complete_step(
    pool: &SqlitePool,
    session: &LearningSession,
    step: &LearningStep,
    result: Option<&str>,
    assisted: bool,
) -> Result<usize> {
    let step_id = step.id.ok_or_else(|| anyhow!("学习步骤尚未持久化"))?;
    let mut transaction = pool.begin().await?;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO learning_step_progress
            (session_id, learning_step_id, status, first_result, assisted, started_at, completed_at)
         VALUES (?1, ?2, 'completed', ?3, ?4, ?5, ?5)
         ON CONFLICT(session_id, learning_step_id) DO UPDATE SET
             status = 'completed', first_result = COALESCE(first_result, excluded.first_result),
             assisted = MAX(assisted, excluded.assisted), completed_at = excluded.completed_at",
    )
    .bind(session.id)
    .bind(step_id)
    .bind(result)
    .bind(i64::from(assisted))
    .bind(now.to_rfc3339())
    .execute(&mut *transaction)
    .await?;
    if step.kind == LearningStepKind::Checkpoint {
        let attempt_rows = sqlx::query(
            "SELECT unit_ids_json, attempt_number, result, assisted
             FROM learning_attempts
             WHERE session_id = ?1 AND learning_step_id = ?2 ORDER BY id",
        )
        .bind(session.id)
        .bind(step_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut outcomes = HashMap::<String, (usize, bool)>::new();
        for row in attempt_rows {
            let unit_ids: Vec<String> = parse_json(row.get("unit_ids_json"), "unit_ids_json")?;
            let attempt_number = row.get::<i64, _>("attempt_number");
            let attempt_result: String = row.get("result");
            let attempt_assisted = row.get::<i64, _>("assisted") != 0;
            for unit_id in unit_ids {
                let outcome = outcomes.entry(unit_id).or_insert((2, false));
                if attempt_number == 1 {
                    outcome.0 = outcome.0.min(result_rank(&attempt_result));
                }
                outcome.1 |= attempt_assisted;
            }
        }
        for local_id in &step.unit_ids {
            let (unit_result, unit_assisted) = outcomes
                .get(local_id)
                .copied()
                .unwrap_or((result.map(result_rank).unwrap_or(0), assisted));
            let delay = if unit_assisted || unit_result < result_rank("correct") {
                Duration::hours(4)
            } else {
                Duration::days(1)
            };
            let due = now + delay;
            sqlx::query(
                "UPDATE knowledge_units
                 SET introduced_at = COALESCE(introduced_at, ?1), due = ?2
                 WHERE note_id = (SELECT note_id FROM learning_plans WHERE id = ?3)
                   AND local_id = ?4",
            )
            .bind(now.to_rfc3339())
            .bind(due.to_rfc3339())
            .bind(session.plan_id)
            .bind(local_id)
            .execute(&mut *transaction)
            .await?;
        }
    }
    let step_count: i64 =
        sqlx::query("SELECT COUNT(*) AS count FROM learning_steps WHERE plan_id = ?1")
            .bind(session.plan_id)
            .fetch_one(&mut *transaction)
            .await?
            .get("count");
    let next = (step.position + 1).min(step_count as usize);
    let completed = next >= step_count as usize;
    sqlx::query(
        "UPDATE learning_sessions
         SET current_step_index = ?2, status = ?3, updated_at = ?4,
             completed_at = CASE WHEN ?3 = 'completed' THEN ?4 ELSE completed_at END
         WHERE id = ?1",
    )
    .bind(session.id)
    .bind(next as i64)
    .bind(if completed { "completed" } else { "active" })
    .bind(now.to_rfc3339())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(next)
}

#[allow(clippy::too_many_arguments)]
pub async fn save_attempt(
    pool: &SqlitePool,
    session_id: i64,
    step_id: i64,
    prompt_id: Option<i64>,
    unit_ids: &[String],
    attempt_number: usize,
    user_answer: &str,
    result: &str,
    score: Option<u32>,
    feedback: &str,
    assisted: bool,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO learning_attempts
            (session_id, learning_step_id, prompt_id, unit_ids_json, attempt_number,
             user_answer, result, score, feedback, assisted, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(session_id)
    .bind(step_id)
    .bind(prompt_id)
    .bind(serde_json::to_string(unit_ids)?)
    .bind(attempt_number as i64)
    .bind(user_answer)
    .bind(result)
    .bind(score.map(i64::from))
    .bind(feedback)
    .bind(i64::from(assisted))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn prompts_for_step(pool: &SqlitePool, step_id: i64) -> Result<Vec<LearningPrompt>> {
    let rows = sqlx::query(
        "SELECT * FROM learning_prompts WHERE learning_step_id = ?1 ORDER BY position, id",
    )
    .bind(step_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(prompt_from_row).collect()
}

pub async fn insert_prompt(pool: &SqlitePool, prompt: &LearningPrompt) -> Result<LearningPrompt> {
    let row = sqlx::query(
        "INSERT INTO learning_prompts
            (learning_step_id, position, unit_ids_json, question_type, question, options_json,
             standard_answer, required_points_json, source_block_ids_json, generation_mode, created_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
         WHERE NOT EXISTS (
             SELECT 1 FROM learning_prompts WHERE learning_step_id = ?1 AND position = ?2
         )
         RETURNING id",
    )
    .bind(prompt.learning_step_id)
    .bind(prompt.position as i64)
    .bind(serde_json::to_string(&prompt.unit_ids)?)
    .bind(prompt.format.as_str())
    .bind(&prompt.question)
    .bind(serde_json::to_string(&prompt.options)?)
    .bind(&prompt.standard_answer)
    .bind(serde_json::to_string(&prompt.required_points)?)
    .bind(serde_json::to_string(&prompt.source_block_ids)?)
    .bind(&prompt.generation_mode)
    .bind(Utc::now().to_rfc3339())
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return prompts_for_step(pool, prompt.learning_step_id)
            .await?
            .into_iter()
            .find(|saved| saved.position == prompt.position)
            .ok_or_else(|| anyhow!("学习题缓存发生并发写入但无法读取"));
    };
    let mut saved = prompt.clone();
    saved.id = Some(row.get("id"));
    Ok(saved)
}

pub async fn fallback_prompt(
    pool: &SqlitePool,
    step: &LearningStep,
    units: &[KnowledgeUnit],
    target_unit_ids: &[String],
    position: usize,
) -> Result<LearningPrompt> {
    let step_id = step.id.ok_or_else(|| anyhow!("学习步骤尚未持久化"))?;
    let selected = step
        .unit_ids
        .iter()
        .filter(|id| target_unit_ids.contains(id))
        .filter_map(|id| units.iter().find(|unit| &unit.local_id == id))
        .collect::<Vec<_>>();
    let primary = selected
        .first()
        .ok_or_else(|| anyhow!("理解检查没有知识单元"))?;
    let required_points = selected
        .iter()
        .flat_map(|unit| unit.required_points.clone())
        .collect::<Vec<_>>();
    let (question, standard_answer) = if selected.len() == 1 {
        let row = sqlx::query(
            "SELECT c.question, c.standard_answer
             FROM card_knowledge_units cku JOIN cards c ON c.id = cku.card_id
             WHERE cku.knowledge_unit_id = ?1 ORDER BY c.id LIMIT 1",
        )
        .bind(primary.id)
        .fetch_optional(pool)
        .await?;
        row.map(|row| (row.get("question"), row.get("standard_answer")))
            .unwrap_or_else(|| {
                (
                    format!("请完成以下学习任务：{}", primary.objective),
                    primary.required_points.join("；"),
                )
            })
    } else {
        let objectives = selected
            .iter()
            .enumerate()
            .map(|(index, unit)| format!("{}. {}", index + 1, unit.objective))
            .collect::<Vec<_>>()
            .join("；");
        (
            format!("请综合回答以下相互关联的学习目标：{objectives}"),
            required_points.join("；"),
        )
    };
    Ok(LearningPrompt {
        id: None,
        learning_step_id: step_id,
        position,
        unit_ids: target_unit_ids.to_vec(),
        format: QuestionFormat::ShortAnswer,
        question,
        options: Vec::new(),
        standard_answer,
        required_points,
        source_block_ids: step.block_ids.clone(),
        generation_mode: "fallback_v3".into(),
    })
}

fn step_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<LearningStep> {
    Ok(LearningStep {
        id: Some(row.get("id")),
        local_id: row.get("local_id"),
        topic_id: row.get("topic_id"),
        topic_title: row.get("topic_title"),
        kind: LearningStepKind::parse(&row.get::<String, _>("kind"))?,
        block_ids: parse_json(row.get("block_ids_json"), "block_ids_json")?,
        unit_ids: parse_json(row.get("unit_ids_json"), "unit_ids_json")?,
        source_step_ids: parse_json(row.get("source_step_ids_json"), "source_step_ids_json")?,
        intent: row
            .get::<Option<String>, _>("intent")
            .map(|value| LearningIntent::parse(&value))
            .transpose()?,
        question_format: row
            .get::<Option<String>, _>("question_format")
            .map(|value| QuestionFormat::parse(&value))
            .transpose()?,
        reason: row.get("reason"),
        position: row.get::<i64, _>("position") as usize,
    })
}

fn block_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ContentBlock> {
    Ok(ContentBlock {
        id: Some(row.get("id")),
        note_id: row.get("note_id"),
        content_hash: row.get("content_hash"),
        local_id: row.get("local_id"),
        kind: ContentBlockKind::parse(&row.get::<String, _>("kind"))?,
        heading_path: parse_json(row.get("heading_path_json"), "heading_path_json")?,
        source_start: row.get::<i64, _>("source_start") as usize,
        source_end: row.get::<i64, _>("source_end") as usize,
        source_text: row.get("source_text"),
        plain_text: row.get("plain_text"),
        position: row.get::<i64, _>("position") as usize,
    })
}

fn prompt_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<LearningPrompt> {
    Ok(LearningPrompt {
        id: Some(row.get("id")),
        learning_step_id: row.get("learning_step_id"),
        position: row.get::<i64, _>("position") as usize,
        unit_ids: parse_json(row.get("unit_ids_json"), "unit_ids_json")?,
        format: QuestionFormat::parse(&row.get::<String, _>("question_type"))?,
        question: row.get("question"),
        options: parse_json(row.get("options_json"), "options_json")?,
        standard_answer: row.get("standard_answer"),
        required_points: parse_json(row.get("required_points_json"), "required_points_json")?,
        source_block_ids: parse_json(row.get("source_block_ids_json"), "source_block_ids_json")?,
        generation_mode: row.get("generation_mode"),
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(value: String, column: &str) -> Result<T> {
    serde_json::from_str(&value).map_err(|error| anyhow!("{column} 数据损坏: {error}"))
}

fn result_rank(result: &str) -> usize {
    match result {
        "incorrect" => 0,
        "partial" => 1,
        "correct" => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, domain::card::Card};

    #[tokio::test]
    async fn learning_session_resumes_and_gates_review() {
        let pool = db::schema::init("sqlite::memory:").await.unwrap();
        let note_id = db::notes::create(&pool, "测试", "# 标题\n\n证据内容")
            .await
            .unwrap();
        db::cards::insert(
            &pool,
            &Card::new(
                note_id,
                "问题".into(),
                "答案".into(),
                Some("证据内容".into()),
            ),
        )
        .await
        .unwrap();
        let note = db::notes::get(&pool, note_id).await.unwrap().unwrap();
        let units = db::knowledge::units_by_note(&pool, note_id).await.unwrap();
        sqlx::query("UPDATE knowledge_units SET introduced_at = NULL WHERE note_id = ?1")
            .bind(note_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            db::dynamic_reviews::due_in_scope(&pool, Utc::now(), &[], Some(note_id))
                .await
                .unwrap()
                .is_empty()
        );
        let plan = prepare_fallback_plan(&pool, &note, &units).await.unwrap();
        let cached = prepare_fallback_plan(&pool, &note, &units).await.unwrap();
        assert_eq!(cached.id, plan.id);
        let plan_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM learning_plans")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("count");
        let block_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM content_blocks")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("count");
        assert_eq!(plan_count, 1);
        assert_eq!(block_count, 2);
        let first = resume_or_start_session(&pool, plan.id.unwrap())
            .await
            .unwrap();
        let resumed = resume_or_start_session(&pool, plan.id.unwrap())
            .await
            .unwrap();
        assert_eq!(first.id, resumed.id);
        assert_eq!(
            learning_progress_state(&pool, note_id).await.unwrap(),
            (true, true)
        );
        assert!(!plan.steps.is_empty());
        let checkpoint = plan
            .steps
            .iter()
            .find(|step| step.kind == LearningStepKind::Checkpoint)
            .unwrap();
        let targets = crate::domain::learning::checkpoint_question_targets(checkpoint, &units);
        let first_prompt = fallback_prompt(&pool, checkpoint, &units, &targets[0], 0)
            .await
            .unwrap();
        let first_prompt = insert_prompt(&pool, &first_prompt).await.unwrap();
        let duplicate = insert_prompt(&pool, &first_prompt).await.unwrap();
        assert_eq!(duplicate.id, first_prompt.id);
        let prompts = prompts_for_step(&pool, checkpoint.id.unwrap())
            .await
            .unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].position, 0);
        assert_eq!(prompts[0].unit_ids, targets[0]);
        assert_eq!(prompts[0].generation_mode, "fallback_v3");
        complete_step(&pool, &first, checkpoint, Some("correct"), false)
            .await
            .unwrap();
        let introduced: Option<String> =
            sqlx::query("SELECT introduced_at FROM knowledge_units WHERE note_id = ?1 LIMIT 1")
                .bind(note_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("introduced_at");
        assert!(introduced.is_some());

        sqlx::query("UPDATE learning_sessions SET status = 'completed' WHERE id = ?1")
            .bind(first.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            learning_progress_state(&pool, note_id).await.unwrap(),
            (true, false)
        );
        assert_eq!(reset_learning_progress(&pool, note_id).await.unwrap(), 1);
        assert_eq!(
            learning_progress_state(&pool, note_id).await.unwrap(),
            (false, false)
        );
        assert_eq!(
            prompts_for_step(&pool, checkpoint.id.unwrap())
                .await
                .unwrap()
                .len(),
            1,
            "重置进度应保留题目缓存"
        );
    }
}
