use anyhow::Result;
use sqlx::SqlitePool;

/// 初始化数据库并应用版本化迁移。`db_path` 形如 `sqlite://path/to/ruiz.db?mode=rwc`。
///
/// 建表与升级统一由 `migrations/` 目录下的 sqlx 迁移管理，
/// 每个迁移只执行一次，由 `_sqlx_migrations` 表记录版本与校验和。
pub async fn init(db_path: &str) -> Result<SqlitePool> {
    let pool = SqlitePool::connect(db_path).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{
        import::ImportedMaterial,
        plan::{MaterialPlan, PlanClaim, PlanUnit},
        workflow::PreparedMaterial,
    };
    use crate::db;
    use sqlx::Row;

    #[tokio::test]
    async fn schema_and_crud_roundtrip() {
        let pool = init("sqlite::memory:").await.expect("内存库建表失败");

        // notes：建、列、查
        let note_id = db::notes::create(&pool, "计算机网络 第三章", "数据链路层内容…")
            .await
            .expect("创建笔记失败");
        let notes = db::notes::list(&pool).await.expect("列出笔记失败");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "计算机网络 第三章");
        assert_eq!(
            db::notes::get(&pool, note_id)
                .await
                .unwrap()
                .unwrap()
                .content,
            "数据链路层内容…"
        );

        // 新版智能导入：知识蓝图、结构化必答点与导入即激活。
        let prepared = PreparedMaterial {
            material: ImportedMaterial {
                title: "计算机网络 第三章".into(),
                content: "数据链路层内容…".into(),
                raw_content: "原始内容".into(),
                summary: "摘要".into(),
                document_type: "concept".into(),
            },
            plan: MaterialPlan {
                summary: "摘要".into(),
                document_type: "concept".into(),
                warnings: vec![],
                claims: vec![PlanClaim {
                    id: "C1".into(),
                    statement: "PPP 使用特定帧格式".into(),
                    importance: "core".into(),
                    evidence: vec!["PPP 帧".into()],
                }],
                units: vec![PlanUnit {
                    id: "K1".into(),
                    topic: "PPP".into(),
                    objective: "能够说明 PPP 帧格式".into(),
                    unit_type: "concept".into(),
                    importance: "core".into(),
                    stage: "foundation".into(),
                    cognitive_action: "recall".into(),
                    required_points: vec!["标志字段为 0x7E".into()],
                    claim_ids: vec!["C1".into()],
                    evidence: vec!["标志字段 0x7E".into()],
                    reason: "核心格式".into(),
                    quick: true,
                    recommended: true,
                    prerequisite_unit_ids: vec![],
                }],
            },
        };
        db::knowledge::save_plan_for_note(&pool, note_id, &prepared)
            .await
            .unwrap();
        let analysis = db::knowledge::analysis_by_note(&pool, note_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(analysis.recommended_count, 1);

        let imported =
            db::knowledge::save_import(&pool, "智能导入", std::slice::from_ref(&prepared))
                .await
                .unwrap();
        assert_eq!(imported.note_ids.len(), 1);
        assert_eq!(imported.activated_units, 1);
        // 推荐单元导入即引入复习队列：到期即出现在动态复习中。
        let due_items = db::dynamic_reviews::due_in_scope(&pool, chrono::Utc::now(), &[], None)
            .await
            .unwrap();
        assert!(!due_items.is_empty());
        let duplicate =
            db::knowledge::save_import(&pool, "智能导入", std::slice::from_ref(&prepared))
                .await
                .expect_err("同一分组不应重复导入相同原始材料");
        assert!(duplicate.to_string().contains("相同原始材料"));

        // 删除笔记时同时清理资料与复习记录。
        db::notes::delete(&pool, note_id).await.unwrap();
        assert!(db::notes::get(&pool, note_id).await.unwrap().is_none());
        let remaining_units: i64 =
            sqlx::query("SELECT COUNT(*) AS count FROM knowledge_units WHERE note_id = ?1")
                .bind(note_id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("count");
        assert_eq!(remaining_units, 0);
    }
}
