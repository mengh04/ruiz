use anyhow::{Result, anyhow};
use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::{
    ai::{plan::PlanUnit, workflow::PreparedMaterial},
    domain::knowledge::{KnowledgeUnit, MaterialAnalysis},
};

#[derive(Debug)]
pub struct ImportSummary {
    pub note_ids: Vec<i64>,
    pub activated_units: usize,
}

/// 原子地保存一次智能导入，避免 AI 工作流中途失败后留下半篇材料。
pub async fn save_import(
    pool: &SqlitePool,
    group_name: &str,
    prepared: &[PreparedMaterial],
) -> Result<ImportSummary> {
    let mut transaction = pool.begin().await?;
    let now = Utc::now().to_rfc3339();
    let group_name = if group_name.trim().is_empty() {
        "未分组"
    } else {
        group_name.trim()
    };
    let group_id: i64 = sqlx::query(
        "INSERT INTO study_groups (name, created_at, updated_at)
         VALUES (?1, ?2, ?2)
         ON CONFLICT(name) DO UPDATE SET updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(group_name)
    .bind(&now)
    .fetch_one(&mut *transaction)
    .await?
    .get("id");

    if let Some(first) = prepared.first() {
        let duplicate = sqlx::query(
            "SELECT n.id
             FROM notes n
             JOIN material_analyses ma ON ma.note_id = n.id
             WHERE n.group_id = ?1 AND ma.source_content = ?2
             LIMIT 1",
        )
        .bind(group_id)
        .bind(&first.material.raw_content)
        .fetch_optional(&mut *transaction)
        .await?
        .is_some();
        if duplicate {
            return Err(anyhow!("相同原始材料已经导入到分组“{group_name}”"));
        }
    }

    let mut note_ids = Vec::with_capacity(prepared.len());
    let mut activated_units = 0;

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

        activated_units += insert_prepared_data(&mut transaction, note_id, item, &now).await?;
    }
    transaction.commit().await?;
    Ok(ImportSummary {
        note_ids,
        activated_units,
    })
}

/// 为旧版笔记补建知识蓝图（复习题目在复习时动态生成）。
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

    for (position, unit) in item.plan.units.iter().enumerate() {
        sqlx::query(
            "INSERT INTO knowledge_units
                (note_id, local_id, topic, objective, unit_type, importance, stage,
                 cognitive_action, required_points_json, claim_ids_json, evidence_json,
                 reason, quick, recommended, generated, stability, difficulty, due,
                 reps, lapses, last_review, introduced_at, prerequisite_ids_json, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, NULL, NULL, ?16, 0, 0, NULL,
                     CASE WHEN ?15 = 1 THEN ?17 ELSE NULL END, ?18, ?19)
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
        // 推荐单元导入即激活并引入复习队列：题目在复习时按知识点动态生成。
        .bind(i64::from(unit.recommended))
        .bind(now)
        .bind(now)
        .bind(serde_json::to_string(&unit.prerequisite_unit_ids)?)
        .bind(position as i64)
        .fetch_one(&mut **transaction)
        .await?
        .get::<i64, _>("id");
    }

    Ok(item.plan.units.iter().filter(|unit| unit.recommended).count())
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

fn unit_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<KnowledgeUnit> {
    let stability: Option<f32> = row.get("stability");
    let difficulty: Option<f32> = row.get("difficulty");
    let memory = match (stability, difficulty) {
        (Some(stability), Some(difficulty)) => Some(fsrs::MemoryState {
            stability,
            difficulty,
        }),
        _ => None,
    };
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
        review_state: crate::domain::dynamic_review::ReviewState {
            memory,
            reps: row.get::<i64, _>("reps") as u32,
            lapses: row.get::<i64, _>("lapses") as u32,
            last_review: row.get("last_review"),
        },
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
