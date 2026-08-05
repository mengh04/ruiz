use std::collections::HashMap;

use anyhow::{Result, anyhow};
use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::{
    ai::{generate::Question, plan::PlanUnit, workflow::PreparedMaterial},
    domain::knowledge::{KnowledgeUnit, MaterialAnalysis},
};

#[derive(Debug)]
pub struct ImportSummary {
    pub note_ids: Vec<i64>,
    pub question_count: usize,
}

/// 原子地保存一次智能导入，避免 AI 工作流中途失败后留下半篇材料。
pub async fn save_import(
    pool: &SqlitePool,
    group_id: i64,
    prepared: &[PreparedMaterial],
) -> Result<ImportSummary> {
    let mut transaction = pool.begin().await?;
    let now = Utc::now().to_rfc3339();
    let mut note_ids = Vec::with_capacity(prepared.len());
    let mut question_count = 0;

    for item in prepared {
        let note_id: i64 = sqlx::query(
            "INSERT INTO notes (group_id, title, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
        )
        .bind(group_id)
        .bind(&item.material.title)
        .bind(&item.material.content)
        .bind(&now)
        .fetch_one(&mut *transaction)
        .await?
        .get("id");
        note_ids.push(note_id);

        question_count += insert_prepared_data(&mut transaction, note_id, item, &now).await?;
    }
    transaction.commit().await?;
    Ok(ImportSummary {
        note_ids,
        question_count,
    })
}

/// 为旧版笔记补建知识蓝图并生成 AI 推荐卡片。
pub async fn save_plan_for_note(
    pool: &SqlitePool,
    note_id: i64,
    prepared: &PreparedMaterial,
) -> Result<usize> {
    let mut transaction = pool.begin().await?;
    let exists: i64 = sqlx::query("SELECT COUNT(*) AS count FROM notes WHERE id = ?1")
        .bind(note_id)
        .fetch_one(&mut *transaction)
        .await?
        .get("count");
    if exists == 0 {
        return Err(anyhow!("要分析的笔记不存在"));
    }
    let now = Utc::now().to_rfc3339();
    let count = insert_prepared_data(&mut transaction, note_id, prepared, &now).await?;
    transaction.commit().await?;
    Ok(count)
}

async fn insert_prepared_data(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    note_id: i64,
    item: &PreparedMaterial,
    now: &str,
) -> Result<usize> {
    let quick_count = item.plan.units.iter().filter(|unit| unit.quick).count();
    let recommended_count = item
        .plan
        .units
        .iter()
        .filter(|unit| unit.recommended)
        .count();
    sqlx::query(
        "INSERT INTO material_analyses
            (note_id, source_content, summary, document_type, warnings_json,
             quick_count, recommended_count, comprehensive_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
    )
    .bind(note_id)
    .bind(&item.material.raw_content)
    .bind(&item.plan.summary)
    .bind(&item.plan.document_type)
    .bind(serde_json::to_string(&item.plan.warnings)?)
    .bind(quick_count as i64)
    .bind(recommended_count as i64)
    .bind(item.plan.units.len() as i64)
    .bind(now)
    .execute(&mut **transaction)
    .await?;

    for (position, claim) in item.plan.claims.iter().enumerate() {
        sqlx::query(
            "INSERT INTO knowledge_claims
                (note_id, local_id, statement, importance, evidence_json, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(note_id)
        .bind(&claim.id)
        .bind(&claim.statement)
        .bind(&claim.importance)
        .bind(serde_json::to_string(&claim.evidence)?)
        .bind(position as i64)
        .execute(&mut **transaction)
        .await?;
    }

    let mut unit_ids = HashMap::new();
    for (position, unit) in item.plan.units.iter().enumerate() {
        let unit_id: i64 = sqlx::query(
            "INSERT INTO knowledge_units
                (note_id, local_id, topic, objective, unit_type, importance, stage,
                 cognitive_action, required_points_json, claim_ids_json, evidence_json,
                 reason, quick, recommended, generated, prerequisite_ids_json, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, 0, ?15, ?16)
             RETURNING id",
        )
        .bind(note_id)
        .bind(&unit.id)
        .bind(&unit.topic)
        .bind(&unit.objective)
        .bind(&unit.unit_type)
        .bind(&unit.importance)
        .bind(&unit.stage)
        .bind(&unit.cognitive_action)
        .bind(serde_json::to_string(&unit.required_points)?)
        .bind(serde_json::to_string(&unit.claim_ids)?)
        .bind(serde_json::to_string(&unit.evidence)?)
        .bind(&unit.reason)
        .bind(i64::from(unit.quick))
        .bind(i64::from(unit.recommended))
        .bind(serde_json::to_string(&unit.prerequisite_unit_ids)?)
        .bind(position as i64)
        .fetch_one(&mut **transaction)
        .await?
        .get("id");
        unit_ids.insert(unit.id.as_str(), unit_id);
    }

    for question in &item.questions {
        let unit_id = *unit_ids
            .get(question.unit_id.as_str())
            .ok_or_else(|| anyhow!("题目引用了不存在的知识单元: {}", question.unit_id))?;
        insert_question(transaction, note_id, unit_id, question, now).await?;
    }
    Ok(item.questions.len())
}

pub async fn analysis_by_note(pool: &SqlitePool, note_id: i64) -> Result<Option<MaterialAnalysis>> {
    let row = sqlx::query("SELECT * FROM material_analyses WHERE note_id = ?1")
        .bind(note_id)
        .fetch_optional(pool)
        .await?;
    row.map(|row| {
        Ok(MaterialAnalysis {
            note_id,
            summary: row.get("summary"),
            document_type: row.get("document_type"),
            warnings: parse_json(row.get::<String, _>("warnings_json"), "warnings_json")?,
            quick_count: row.get::<i64, _>("quick_count") as usize,
            recommended_count: row.get::<i64, _>("recommended_count") as usize,
            comprehensive_count: row.get::<i64, _>("comprehensive_count") as usize,
        })
    })
    .transpose()
}

pub async fn units_by_note(pool: &SqlitePool, note_id: i64) -> Result<Vec<KnowledgeUnit>> {
    let rows = sqlx::query("SELECT * FROM knowledge_units WHERE note_id = ?1 ORDER BY position")
        .bind(note_id)
        .fetch_all(pool)
        .await?;
    rows.iter().map(unit_from_row).collect()
}

pub async fn save_generated_questions(
    pool: &SqlitePool,
    note_id: i64,
    questions: &[Question],
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let now = Utc::now().to_rfc3339();
    for question in questions {
        let unit_id: i64 = sqlx::query(
            "SELECT id FROM knowledge_units
             WHERE note_id = ?1 AND local_id = ?2 AND generated = 0",
        )
        .bind(note_id)
        .bind(&question.unit_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| anyhow!("知识单元不存在或已经生成题目: {}", question.unit_id))?
        .get("id");
        insert_question(&mut transaction, note_id, unit_id, question, &now).await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn insert_question(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    note_id: i64,
    unit_id: i64,
    question: &Question,
    now: &str,
) -> Result<()> {
    let card_id: i64 = sqlx::query(
        "INSERT INTO cards
            (note_id, question, standard_answer, source_excerpt,
             stability, difficulty, due, reps, lapses, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, 0, 0, ?5, ?5)
         RETURNING id",
    )
    .bind(note_id)
    .bind(&question.question)
    .bind(&question.standard_answer)
    .bind(&question.source_excerpt)
    .bind(now)
    .fetch_one(&mut **transaction)
    .await?
    .get("id");
    sqlx::query("INSERT INTO card_knowledge_units (card_id, knowledge_unit_id) VALUES (?1, ?2)")
        .bind(card_id)
        .bind(unit_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE knowledge_units SET generated = 1 WHERE id = ?1")
        .bind(unit_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn unit_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<KnowledgeUnit> {
    Ok(KnowledgeUnit {
        id: row.get("id"),
        note_id: row.get("note_id"),
        local_id: row.get("local_id"),
        topic: row.get("topic"),
        objective: row.get("objective"),
        unit_type: row.get("unit_type"),
        importance: row.get("importance"),
        stage: row.get("stage"),
        cognitive_action: row.get("cognitive_action"),
        required_points: parse_json(
            row.get::<String, _>("required_points_json"),
            "required_points_json",
        )?,
        claim_ids: parse_json(row.get::<String, _>("claim_ids_json"), "claim_ids_json")?,
        evidence: parse_json(row.get::<String, _>("evidence_json"), "evidence_json")?,
        reason: row.get("reason"),
        quick: row.get::<i64, _>("quick") != 0,
        recommended: row.get::<i64, _>("recommended") != 0,
        generated: row.get::<i64, _>("generated") != 0,
        prerequisite_unit_ids: parse_json(
            row.get::<String, _>("prerequisite_ids_json"),
            "prerequisite_ids_json",
        )?,
        position: row.get::<i64, _>("position") as usize,
    })
}

fn parse_json<T: serde::de::DeserializeOwned>(value: String, column: &str) -> Result<T> {
    serde_json::from_str(&value).map_err(|error| anyhow!("{column} 数据损坏: {error}"))
}

impl From<&KnowledgeUnit> for PlanUnit {
    fn from(unit: &KnowledgeUnit) -> Self {
        Self {
            id: unit.local_id.clone(),
            topic: unit.topic.clone(),
            objective: unit.objective.clone(),
            unit_type: unit.unit_type.clone(),
            importance: unit.importance.clone(),
            stage: unit.stage.clone(),
            cognitive_action: unit.cognitive_action.clone(),
            required_points: unit.required_points.clone(),
            claim_ids: unit.claim_ids.clone(),
            evidence: unit.evidence.clone(),
            reason: unit.reason.clone(),
            quick: unit.quick,
            recommended: unit.recommended,
            prerequisite_unit_ids: unit.prerequisite_unit_ids.clone(),
        }
    }
}
