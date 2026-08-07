-- 0001：初始 schema（合并了旧版手写迁移补的列：knowledge_units.introduced_at 与复习索引）。
-- 使用 IF NOT EXISTS 保持幂等，兼容测试期遗留的旧库文件。

CREATE TABLE IF NOT EXISTS study_groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id   INTEGER REFERENCES study_groups(id),
    title      TEXT NOT NULL,
    content    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS material_analyses (
    note_id              INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    source_content       TEXT NOT NULL,
    summary              TEXT NOT NULL,
    document_type        TEXT NOT NULL,
    warnings_json        TEXT NOT NULL DEFAULT '[]',
    quick_count          INTEGER NOT NULL,
    recommended_count    INTEGER NOT NULL,
    comprehensive_count  INTEGER NOT NULL,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS knowledge_claims (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id        INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    local_id       TEXT NOT NULL,
    statement      TEXT NOT NULL,
    importance     TEXT NOT NULL,
    evidence_json  TEXT NOT NULL,
    position       INTEGER NOT NULL,
    UNIQUE(note_id, local_id)
);
CREATE INDEX IF NOT EXISTS idx_claims_note ON knowledge_claims(note_id, position);

CREATE TABLE IF NOT EXISTS knowledge_units (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id                INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    local_id               TEXT NOT NULL,
    topic                  TEXT NOT NULL,
    objective              TEXT NOT NULL,
    unit_type              TEXT NOT NULL,
    importance             TEXT NOT NULL,
    stage                  TEXT NOT NULL,
    cognitive_action       TEXT NOT NULL,
    required_points_json   TEXT NOT NULL,
    claim_ids_json         TEXT NOT NULL,
    evidence_json          TEXT NOT NULL,
    reason                 TEXT NOT NULL,
    quick                  INTEGER NOT NULL DEFAULT 0,
    recommended            INTEGER NOT NULL DEFAULT 0,
    generated              INTEGER NOT NULL DEFAULT 0,
    stability              REAL,
    difficulty             REAL,
    due                    TEXT NOT NULL,
    reps                   INTEGER NOT NULL DEFAULT 0,
    lapses                 INTEGER NOT NULL DEFAULT 0,
    last_review            TEXT,
    introduced_at          TEXT,
    prerequisite_ids_json  TEXT NOT NULL DEFAULT '[]',
    position               INTEGER NOT NULL,
    UNIQUE(note_id, local_id)
);
CREATE INDEX IF NOT EXISTS idx_units_note ON knowledge_units(note_id, position);
CREATE INDEX IF NOT EXISTS idx_knowledge_units_due
ON knowledge_units(generated, due);
CREATE INDEX IF NOT EXISTS idx_knowledge_units_introduced_due
ON knowledge_units(generated, introduced_at, due);

CREATE TABLE IF NOT EXISTS review_prompts (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_unit_id    INTEGER NOT NULL REFERENCES knowledge_units(id) ON DELETE CASCADE,
    question_type        TEXT NOT NULL,
    mastery_band         TEXT NOT NULL,
    question             TEXT NOT NULL,
    options_json         TEXT NOT NULL DEFAULT '[]',
    standard_answer      TEXT NOT NULL,
    required_points_json TEXT NOT NULL DEFAULT '[]',
    source_excerpt       TEXT,
    generation_mode      TEXT NOT NULL,
    created_at           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_review_prompts_unit
ON review_prompts(knowledge_unit_id, created_at DESC);

CREATE TABLE IF NOT EXISTS review_attempts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_unit_id INTEGER NOT NULL REFERENCES knowledge_units(id) ON DELETE CASCADE,
    prompt_id         INTEGER REFERENCES review_prompts(id) ON DELETE SET NULL,
    user_answer       TEXT NOT NULL,
    ai_feedback       TEXT NOT NULL,
    rating            INTEGER NOT NULL,
    reviewed_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_review_attempts_unit
ON review_attempts(knowledge_unit_id, reviewed_at DESC);

CREATE TABLE IF NOT EXISTS content_blocks (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id           INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    content_hash      TEXT NOT NULL,
    local_id          TEXT NOT NULL,
    kind              TEXT NOT NULL,
    heading_path_json TEXT NOT NULL DEFAULT '[]',
    source_start      INTEGER NOT NULL,
    source_end        INTEGER NOT NULL,
    source_text       TEXT NOT NULL,
    plain_text        TEXT NOT NULL,
    position          INTEGER NOT NULL,
    UNIQUE(note_id, content_hash, local_id)
);
CREATE INDEX IF NOT EXISTS idx_content_blocks_note
ON content_blocks(note_id, content_hash, position);

CREATE TABLE IF NOT EXISTS knowledge_unit_sources (
    knowledge_unit_id INTEGER NOT NULL REFERENCES knowledge_units(id) ON DELETE CASCADE,
    content_block_id  INTEGER NOT NULL REFERENCES content_blocks(id) ON DELETE CASCADE,
    relevance         TEXT NOT NULL,
    position          INTEGER NOT NULL,
    PRIMARY KEY (knowledge_unit_id, content_block_id)
);

CREATE TABLE IF NOT EXISTS learning_plans (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id           INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    content_hash      TEXT NOT NULL,
    plan_version      INTEGER NOT NULL,
    summary           TEXT NOT NULL,
    estimated_minutes INTEGER NOT NULL,
    generation_mode   TEXT NOT NULL,
    topics_json       TEXT NOT NULL DEFAULT '[]',
    created_at        TEXT NOT NULL,
    UNIQUE(note_id, content_hash, plan_version)
);

CREATE TABLE IF NOT EXISTS learning_steps (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id              INTEGER NOT NULL REFERENCES learning_plans(id) ON DELETE CASCADE,
    local_id             TEXT NOT NULL,
    topic_id             TEXT NOT NULL,
    topic_title          TEXT NOT NULL,
    kind                 TEXT NOT NULL,
    block_ids_json       TEXT NOT NULL DEFAULT '[]',
    unit_ids_json        TEXT NOT NULL DEFAULT '[]',
    source_step_ids_json TEXT NOT NULL DEFAULT '[]',
    intent               TEXT,
    question_format      TEXT,
    reason               TEXT,
    position             INTEGER NOT NULL,
    UNIQUE(plan_id, local_id)
);

CREATE TABLE IF NOT EXISTS learning_prompts (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    learning_step_id      INTEGER NOT NULL REFERENCES learning_steps(id) ON DELETE CASCADE,
    position              INTEGER NOT NULL DEFAULT 0,
    unit_ids_json         TEXT NOT NULL DEFAULT '[]',
    question_type         TEXT NOT NULL,
    question              TEXT NOT NULL,
    options_json          TEXT NOT NULL DEFAULT '[]',
    standard_answer       TEXT NOT NULL,
    required_points_json  TEXT NOT NULL DEFAULT '[]',
    source_block_ids_json TEXT NOT NULL DEFAULT '[]',
    generation_mode       TEXT NOT NULL,
    created_at            TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS learning_sessions (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id            INTEGER NOT NULL REFERENCES learning_plans(id) ON DELETE CASCADE,
    status             TEXT NOT NULL,
    current_step_index INTEGER NOT NULL DEFAULT 0,
    started_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    completed_at       TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_learning_sessions_active_plan
ON learning_sessions(plan_id)
WHERE status IN ('not_started', 'active', 'paused');

CREATE TABLE IF NOT EXISTS learning_step_progress (
    session_id       INTEGER NOT NULL REFERENCES learning_sessions(id) ON DELETE CASCADE,
    learning_step_id INTEGER NOT NULL REFERENCES learning_steps(id) ON DELETE CASCADE,
    status           TEXT NOT NULL,
    first_result     TEXT,
    assisted         INTEGER NOT NULL DEFAULT 0,
    runtime_json     TEXT NOT NULL DEFAULT '{}',
    started_at       TEXT,
    completed_at     TEXT,
    PRIMARY KEY (session_id, learning_step_id)
);

CREATE TABLE IF NOT EXISTS learning_attempts (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       INTEGER NOT NULL REFERENCES learning_sessions(id) ON DELETE CASCADE,
    learning_step_id INTEGER NOT NULL REFERENCES learning_steps(id) ON DELETE CASCADE,
    prompt_id        INTEGER REFERENCES learning_prompts(id) ON DELETE SET NULL,
    unit_ids_json    TEXT NOT NULL DEFAULT '[]',
    attempt_number   INTEGER NOT NULL,
    user_answer      TEXT NOT NULL,
    result           TEXT NOT NULL,
    score            INTEGER,
    feedback         TEXT NOT NULL,
    assisted         INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT NOT NULL
);
