use anyhow::Result;
use sqlx::SqlitePool;

/// 建表语句（idempotent，可重复执行）。
/// `cards.stability / difficulty` 可空：NULL = 新卡（尚未首次复习）。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS notes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT NOT NULL,
    content    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cards (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id         INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    question        TEXT NOT NULL,
    standard_answer TEXT NOT NULL,
    source_excerpt  TEXT,
    stability       REAL,
    difficulty      REAL,
    due             TEXT NOT NULL,
    reps            INTEGER NOT NULL DEFAULT 0,
    lapses          INTEGER NOT NULL DEFAULT 0,
    last_review     TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cards_note ON cards(note_id);
CREATE INDEX IF NOT EXISTS idx_cards_due ON cards(due);

CREATE TABLE IF NOT EXISTS reviews (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    card_id     INTEGER NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    user_answer TEXT NOT NULL,
    ai_feedback TEXT NOT NULL,
    rating      INTEGER NOT NULL,
    reviewed_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reviews_card ON reviews(card_id);
"#;

/// 初始化数据库并建表。`db_path` 形如 `sqlite://path/to/ruiz.db?mode=rwc`。
pub async fn init(db_path: &str) -> Result<SqlitePool> {
    let pool = SqlitePool::connect(db_path).await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{card::Card, review::Rating};
    use crate::{db, scheduler::Scheduler};

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

        // cards：插入、按笔记查、到期队列
        let card = Card::new(
            note_id,
            "PPP 帧格式？".into(),
            "标志字段 0x7E…".into(),
            None,
        );
        let card_id = db::cards::insert(&pool, &card).await.expect("插入卡片失败");
        let cards = db::cards::by_note(&pool, note_id).await.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].question, "PPP 帧格式？");
        // 新卡 due 是当前时间，应进入复习队列
        let due = db::cards::due(&pool, chrono::Utc::now()).await.unwrap();
        assert_eq!(due.len(), 1);
        assert!(due[0].memory.is_none(), "新卡不应有记忆状态");

        // FSRS：新卡 Good 后的调度 + 落库
        let scheduler = Scheduler::new();
        let next = scheduler.next_states(None, 0).expect("FSRS 计算失败");
        let state = Scheduler::state_for(Rating::Good, &next);
        let due_at = Scheduler::due_date(state);
        assert!(state.memory.stability > 0.0, "Good 后 stability 应大于 0");
        db::cards::update_schedule(&pool, card_id, state.memory, due_at, 1, 0)
            .await
            .unwrap();

        // 更新后再查：有记忆状态、reps=1
        let updated = db::cards::get(&pool, card_id).await.unwrap().unwrap();
        assert!(updated.memory.is_some());
        assert_eq!(updated.reps, 1);

        // reviews：记录作答历史
        db::reviews::insert(&pool, card_id, "我的答案", "判官反馈", Rating::Good)
            .await
            .expect("插入复习记录失败");
        let reviews = db::reviews::by_card(&pool, card_id).await.unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].rating, Rating::Good);

        // 删除笔记时同时清理卡片和复习记录。
        db::notes::delete(&pool, note_id).await.unwrap();
        assert!(db::notes::get(&pool, note_id).await.unwrap().is_none());
        assert!(db::cards::get(&pool, card_id).await.unwrap().is_none());
        assert!(
            db::reviews::by_card(&pool, card_id)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
