// ============================================================================
// SQLite Schema Definitions
// Based on TypeScript: /opencode/packages/opencode/src/session/session.sql.ts
// ============================================================================

/// Sessions table - stores session metadata
pub const CREATE_SESSIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    parent_id TEXT,
    slug TEXT NOT NULL,
    directory TEXT NOT NULL,
    title TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    share_url TEXT,
    
    -- Summary fields
    summary_additions INTEGER DEFAULT 0,
    summary_deletions INTEGER DEFAULT 0,
    summary_files INTEGER DEFAULT 0,
    summary_diffs TEXT,
    
    -- Revert info (JSON)
    revert TEXT,
    
    -- Permission ruleset (JSON)
    permission TEXT,

    -- Arbitrary session metadata (JSON)
    metadata TEXT,
    
    -- Usage stats
    usage_input_tokens INTEGER DEFAULT 0,
    usage_context_tokens INTEGER DEFAULT 0,
    usage_output_tokens INTEGER DEFAULT 0,
    usage_reasoning_tokens INTEGER DEFAULT 0,
    usage_cache_write_tokens INTEGER DEFAULT 0,
    usage_cache_read_tokens INTEGER DEFAULT 0,
    usage_cache_miss_tokens INTEGER DEFAULT 0,
    usage_total_cost REAL DEFAULT 0.0,
    
    -- Status
    status TEXT NOT NULL DEFAULT 'active',
    
    -- Timestamps (milliseconds since epoch)
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    time_compacting INTEGER,
    time_archived INTEGER
);
"#;

/// Messages table - stores message metadata
pub const CREATE_MESSAGES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at INTEGER NOT NULL,

    -- Provider/model info
    provider_id TEXT,
    model_id TEXT,

    -- Token usage
    tokens_input INTEGER DEFAULT 0,
    tokens_context INTEGER DEFAULT 0,
    tokens_output INTEGER DEFAULT 0,
    tokens_reasoning INTEGER DEFAULT 0,
    tokens_cache_read INTEGER DEFAULT 0,
    tokens_cache_miss INTEGER DEFAULT 0,
    tokens_cache_write INTEGER DEFAULT 0,
    cost REAL DEFAULT 0.0,

    -- LLM finish reason (e.g. "stop", "tool-calls")
    finish TEXT,

    -- Arbitrary message metadata (JSON)
    metadata TEXT,

    -- Complete message data (JSON)
    data TEXT,

    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Parts table - stores message parts (text, tool calls, etc.)
pub const CREATE_PARTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS parts (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    
    -- Part type
    part_type TEXT NOT NULL,
    
    -- Text content
    text TEXT,
    
    -- Tool call fields
    tool_name TEXT,
    tool_call_id TEXT,
    tool_arguments TEXT,
    tool_result TEXT,
    tool_error TEXT,
    tool_status TEXT,
    
    -- File fields
    file_url TEXT,
    file_filename TEXT,
    file_mime TEXT,
    
    -- Reasoning fields
    reasoning TEXT,
    
    -- Sort order
    sort_order INTEGER NOT NULL DEFAULT 0,
    
    -- Complete part data (JSON)
    data TEXT,
    
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Todos table - stores session todos
pub const CREATE_TODOS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS todos (
    session_id TEXT NOT NULL,
    todo_id TEXT NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    priority TEXT NOT NULL DEFAULT 'medium',
    position INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    
    PRIMARY KEY (session_id, todo_id),
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Permissions table - stores project-level permissions
pub const CREATE_PERMISSIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS permissions (
    project_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    data TEXT NOT NULL
);
"#;

/// Session shares table - stores share info for sessions
pub const CREATE_SESSION_SHARES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS session_shares (
    session_id TEXT PRIMARY KEY,
    id TEXT NOT NULL,
    secret TEXT NOT NULL,
    url TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    
    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
"#;

/// Memory records table - stores structured memory observations.
pub const CREATE_MEMORY_RECORDS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_records (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'candidate',
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    trigger_conditions TEXT,
    normalized_facts TEXT,
    boundaries TEXT,
    confidence REAL,
    source_session_id TEXT,
    workspace_identity TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_validated_at INTEGER,
    expires_at INTEGER,
    derived_skill_name TEXT,
    linked_skill_name TEXT,
    validation_status TEXT NOT NULL DEFAULT 'pending'
);
"#;

/// Memory evidence table - normalized provenance rows for each record.
pub const CREATE_MEMORY_EVIDENCE_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_evidence (
    memory_id TEXT NOT NULL,
    evidence_index INTEGER NOT NULL,
    session_id TEXT,
    message_id TEXT,
    tool_call_id TEXT,
    stage_id TEXT,
    note TEXT,
    PRIMARY KEY (memory_id, evidence_index),
    FOREIGN KEY (memory_id) REFERENCES memory_records(id) ON DELETE CASCADE
);
"#;

/// Memory validation runs table - stores validation reports.
pub const CREATE_MEMORY_VALIDATION_RUNS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_validation_runs (
    run_id TEXT PRIMARY KEY,
    memory_id TEXT,
    status TEXT NOT NULL,
    issues TEXT,
    checked_at INTEGER NOT NULL,
    FOREIGN KEY (memory_id) REFERENCES memory_records(id) ON DELETE SET NULL
);
"#;

/// Memory conflicts table - reserved for future contradiction tracking.
pub const CREATE_MEMORY_CONFLICTS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_conflicts (
    id TEXT PRIMARY KEY,
    left_memory_id TEXT,
    right_memory_id TEXT,
    conflict_kind TEXT,
    detail TEXT,
    detected_at INTEGER NOT NULL,
    FOREIGN KEY (left_memory_id) REFERENCES memory_records(id) ON DELETE SET NULL,
    FOREIGN KEY (right_memory_id) REFERENCES memory_records(id) ON DELETE SET NULL
);
"#;

/// Memory consolidation runs table - stores consolidation job outcomes.
pub const CREATE_MEMORY_CONSOLIDATION_RUNS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_consolidation_runs (
    run_id TEXT PRIMARY KEY,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    merged_count INTEGER NOT NULL DEFAULT 0,
    promoted_count INTEGER NOT NULL DEFAULT 0,
    conflict_count INTEGER NOT NULL DEFAULT 0
);
"#;

/// Memory retrieval log table - stores retrieval/read observations.
pub const CREATE_MEMORY_RETRIEVAL_LOG_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_retrieval_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT,
    query TEXT,
    stage TEXT,
    scopes TEXT,
    retrieved_count INTEGER NOT NULL DEFAULT 0,
    used_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
"#;

/// Memory rule packs table - reserved for later validation/consolidation rule packs.
pub const CREATE_MEMORY_RULE_PACKS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_rule_packs (
    id TEXT PRIMARY KEY,
    rule_pack_kind TEXT NOT NULL,
    version TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

/// Memory rule hits table - reserved for later rule execution evidence.
pub const CREATE_MEMORY_RULE_HITS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS memory_rule_hits (
    id TEXT PRIMARY KEY,
    rule_pack_id TEXT,
    memory_id TEXT,
    run_id TEXT,
    hit_kind TEXT NOT NULL,
    detail TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (rule_pack_id) REFERENCES memory_rule_packs(id) ON DELETE SET NULL,
    FOREIGN KEY (memory_id) REFERENCES memory_records(id) ON DELETE SET NULL
);
"#;

/// Create indexes for better query performance
pub const CREATE_INDEXES: &str = r#"
-- Session indexes
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id);
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_id);
CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);

-- Message indexes
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);

-- Part indexes
CREATE INDEX IF NOT EXISTS idx_parts_message ON parts(message_id);
CREATE INDEX IF NOT EXISTS idx_parts_session ON parts(session_id);
CREATE INDEX IF NOT EXISTS idx_parts_order ON parts(sort_order);

-- Todo indexes
CREATE INDEX IF NOT EXISTS idx_todos_session ON todos(session_id);
CREATE INDEX IF NOT EXISTS idx_todos_status ON todos(status);

-- Memory indexes
CREATE INDEX IF NOT EXISTS idx_memory_records_scope ON memory_records(scope);
CREATE INDEX IF NOT EXISTS idx_memory_records_kind ON memory_records(kind);
CREATE INDEX IF NOT EXISTS idx_memory_records_status ON memory_records(status);
CREATE INDEX IF NOT EXISTS idx_memory_records_updated ON memory_records(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_records_workspace ON memory_records(workspace_identity);
CREATE INDEX IF NOT EXISTS idx_memory_records_session ON memory_records(source_session_id);
CREATE INDEX IF NOT EXISTS idx_memory_evidence_memory ON memory_evidence(memory_id);
CREATE INDEX IF NOT EXISTS idx_memory_validation_runs_memory ON memory_validation_runs(memory_id);
CREATE INDEX IF NOT EXISTS idx_memory_retrieval_log_session ON memory_retrieval_log(session_id);
"#;

/// Skill evolution proposals table — evidence-backed skill lifecycle proposals
/// generated from methodology candidates after consolidation.
pub const CREATE_SKILL_EVOLUTION_PROPOSALS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS skill_evolution_proposals (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    memory_record_ids_json TEXT NOT NULL,
    linked_skill_name TEXT,
    proposal_kind TEXT NOT NULL,
    title TEXT NOT NULL,
    rationale TEXT NOT NULL,
    suggested_changes_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    evidence_hash TEXT NOT NULL UNIQUE,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
"#;

pub const CREATE_SKILL_EVOLUTION_PROPOSALS_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_skill_evolution_proposals_status
ON skill_evolution_proposals(status);

CREATE INDEX IF NOT EXISTS idx_skill_evolution_proposals_linked_skill
ON skill_evolution_proposals(linked_skill_name);

CREATE INDEX IF NOT EXISTS idx_skill_evolution_proposals_session
ON skill_evolution_proposals(session_id);
"#;

/// Add finish column to messages table for existing databases.
/// New databases get it from CREATE TABLE; this handles upgrades.
pub const ADD_MESSAGES_FINISH_COLUMN: &str = "ALTER TABLE messages ADD COLUMN finish TEXT";
pub const ADD_SESSIONS_METADATA_COLUMN: &str = "ALTER TABLE sessions ADD COLUMN metadata TEXT";
pub const ADD_MESSAGES_METADATA_COLUMN: &str = "ALTER TABLE messages ADD COLUMN metadata TEXT";
pub const ADD_SESSIONS_USAGE_CONTEXT_COLUMN: &str =
    "ALTER TABLE sessions ADD COLUMN usage_context_tokens INTEGER DEFAULT 0";
pub const ADD_MESSAGES_TOKENS_CONTEXT_COLUMN: &str =
    "ALTER TABLE messages ADD COLUMN tokens_context INTEGER DEFAULT 0";
pub const ADD_SESSIONS_USAGE_CACHE_MISS_COLUMN: &str =
    "ALTER TABLE sessions ADD COLUMN usage_cache_miss_tokens INTEGER DEFAULT 0";
pub const ADD_MESSAGES_TOKENS_CACHE_MISS_COLUMN: &str =
    "ALTER TABLE messages ADD COLUMN tokens_cache_miss INTEGER DEFAULT 0";

/// All migration statements to run
pub const ALL_MIGRATIONS: &[&str] = &[
    CREATE_SESSIONS_TABLE,
    CREATE_MESSAGES_TABLE,
    CREATE_PARTS_TABLE,
    CREATE_TODOS_TABLE,
    CREATE_PERMISSIONS_TABLE,
    CREATE_SESSION_SHARES_TABLE,
    CREATE_MEMORY_RECORDS_TABLE,
    CREATE_MEMORY_EVIDENCE_TABLE,
    CREATE_MEMORY_VALIDATION_RUNS_TABLE,
    CREATE_MEMORY_CONFLICTS_TABLE,
    CREATE_MEMORY_CONSOLIDATION_RUNS_TABLE,
    CREATE_MEMORY_RETRIEVAL_LOG_TABLE,
    CREATE_MEMORY_RULE_PACKS_TABLE,
    CREATE_MEMORY_RULE_HITS_TABLE,
    CREATE_SKILL_EVOLUTION_PROPOSALS_TABLE,
    CREATE_INDEXES,
    CREATE_SKILL_EVOLUTION_PROPOSALS_INDEXES,
    ADD_MESSAGES_FINISH_COLUMN,
    ADD_SESSIONS_METADATA_COLUMN,
    ADD_MESSAGES_METADATA_COLUMN,
    ADD_SESSIONS_USAGE_CONTEXT_COLUMN,
    ADD_MESSAGES_TOKENS_CONTEXT_COLUMN,
    ADD_SESSIONS_USAGE_CACHE_MISS_COLUMN,
    ADD_MESSAGES_TOKENS_CACHE_MISS_COLUMN,
];
