use agendao_types::{MessagePart, PartType};
use anyhow::Result;
use serde_json::Value;
use sqlx::sqlite::{SqliteConnection, SqlitePool, SqlitePoolOptions};
use sqlx::{FromRow, Sqlite, Transaction};
use std::future::Future;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Database connection error: {0}")]
    ConnectionError(String),

    #[error("Migration error: {0}")]
    MigrationError(String),

    #[error("Query error: {0}")]
    QueryError(String),

    #[error("Transaction error: {0}")]
    TransactionError(String),
}

pub struct Database {
    pool: SqlitePool,
}

pub type SqliteTransaction<'a> = Transaction<'a, Sqlite>;
type MemoryRecordSignatureRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

impl Database {
    pub async fn new() -> Result<Self, DatabaseError> {
        let db_path = Self::get_database_path()?;

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DatabaseError::ConnectionError(e.to_string()))?;
        }

        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        info!("Connecting to database at {}", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await
            .map_err(|e| DatabaseError::ConnectionError(e.to_string()))?;

        // WAL mode allows concurrent reads during writes; NORMAL sync reduces fsync overhead.
        if let Err(e) = sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await {
            warn!("failed to set journal_mode=WAL: {}", e);
        }
        if let Err(e) = sqlx::query("PRAGMA synchronous=NORMAL")
            .execute(&pool)
            .await
        {
            warn!("failed to set synchronous=NORMAL: {}", e);
        }

        let db = Self { pool };
        db.run_migrations().await?;

        Ok(db)
    }

    pub async fn in_memory() -> Result<Self, DatabaseError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(|e| DatabaseError::ConnectionError(e.to_string()))?;

        let db = Self { pool };
        db.run_migrations().await?;

        Ok(db)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn begin(&self) -> Result<SqliteTransaction<'_>, DatabaseError> {
        self.pool
            .begin()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))
    }

    pub async fn transaction<F, T, Fut>(&self, f: F) -> Result<T, DatabaseError>
    where
        F: FnOnce(&mut SqliteTransaction<'_>) -> Fut,
        Fut: Future<Output = Result<T, DatabaseError>>,
    {
        let mut tx = self.begin().await?;
        let result = f(&mut tx).await?;
        tx.commit()
            .await
            .map_err(|e| DatabaseError::TransactionError(e.to_string()))?;
        Ok(result)
    }

    pub async fn get_connection(&self) -> Result<SqliteConnection, DatabaseError> {
        self.pool
            .acquire()
            .await
            .map(|conn| conn.detach())
            .map_err(|e| DatabaseError::ConnectionError(e.to_string()))
    }

    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        info!("Running database migrations");

        // DDL 收进单个事务：26 条 CREATE/ALTER 各自隐式提交会产生 26 次
        // fsync，是启动数据库阶段的主要 IO 开销；合并后一次提交即可。
        // ALTER ADD COLUMN 的 "duplicate column" 容错语义保持不变。
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        for migration in crate::schema::ALL_MIGRATIONS {
            match sqlx::query(migration).execute(&mut *tx).await {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    // ALTER TABLE ADD COLUMN fails with "duplicate column" on
                    // databases that already have the column — safe to ignore.
                    if msg.contains("duplicate column") {
                        continue;
                    }
                    return Err(DatabaseError::MigrationError(msg));
                }
            }
        }
        tx.commit()
            .await
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;

        // Gate the one-time tool-call input migration so later starts avoid a
        // full messages-table scan and per-row JSON decoding.
        let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        if user_version < 1 {
            self.run_tool_call_input_data_migration().await?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&self.pool)
                .await
                .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        }

        // v2：回填 memory_records.signature。该列由 ALL_MIGRATIONS 中的
        // ALTER 添加（旧库）或 CREATE TABLE 自带（新库）；这里一次性为列
        // 出现前写入的行计算 canonical signature，幂等且只跑一次。
        if user_version < 2 {
            self.backfill_memory_record_signatures().await?;
            sqlx::query("PRAGMA user_version = 2")
                .execute(&self.pool)
                .await
                .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        }

        Ok(())
    }

    async fn backfill_memory_record_signatures(&self) -> Result<(), DatabaseError> {
        let rows: Vec<MemoryRecordSignatureRow> = sqlx::query_as(
            r#"SELECT id, scope, title, summary, trigger_conditions, normalized_facts
                   FROM memory_records
                   WHERE signature IS NULL"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        if rows.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        for (id, scope, title, summary, trigger_conditions, normalized_facts) in rows {
            let trigger_conditions: Vec<String> = trigger_conditions
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default();
            let normalized_facts: Vec<String> = normalized_facts
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default();
            let signature = crate::repository::memory_record_signature_parts(
                &scope,
                &title,
                &summary,
                &trigger_conditions,
                &normalized_facts,
            );
            sqlx::query("UPDATE memory_records SET signature = ? WHERE id = ?")
                .bind(signature)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
        Ok(())
    }

    async fn run_tool_call_input_data_migration(&self) -> Result<(), DatabaseError> {
        #[derive(Debug, FromRow)]
        struct MessageRow {
            id: String,
            data: Option<String>,
        }

        let rows = sqlx::query_as::<_, MessageRow>(
            r#"SELECT id, data
               FROM messages
               WHERE role = 'assistant' AND data IS NOT NULL"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;

        let mut updated_rows = 0usize;
        let mut invalid_reroutes = 0usize;

        for row in rows {
            let Some(data) = row.data else {
                continue;
            };

            let mut parts: Vec<MessagePart> = match serde_json::from_str(&data) {
                Ok(parts) => parts,
                Err(error) => {
                    warn!(
                        message_id = %row.id,
                        %error,
                        "skipping data migration for message with invalid parts JSON"
                    );
                    continue;
                }
            };

            let mut changed = false;
            for part in &mut parts {
                if let PartType::ToolCall { name, input, .. } = &mut part.part_type {
                    let (sanitized, rerouted_invalid) =
                        sanitize_tool_call_input_for_storage(name, input);
                    if *input != sanitized {
                        *input = sanitized;
                        changed = true;
                    }
                    if rerouted_invalid {
                        invalid_reroutes += 1;
                    }
                }
            }

            if !changed {
                continue;
            }

            let next_data = serde_json::to_string(&parts)
                .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
            sqlx::query("UPDATE messages SET data = ? WHERE id = ?")
                .bind(next_data)
                .bind(&row.id)
                .execute(&self.pool)
                .await
                .map_err(|e| DatabaseError::MigrationError(e.to_string()))?;
            updated_rows += 1;
        }

        if updated_rows > 0 || invalid_reroutes > 0 {
            info!(
                updated_rows,
                invalid_reroutes, "tool call input data migration complete"
            );
        }

        Ok(())
    }

    fn get_database_path() -> Result<PathBuf, DatabaseError> {
        // 用户级数据库统一收在 agendao_home（~/.agendao，土律·单点权威）。
        Ok(agendao_util::agendao_home().join("agendao.db"))
    }
}

fn invalid_tool_payload_for_storage(tool_name: &str, error: &str, received_args: Value) -> Value {
    serde_json::json!({
        "tool": tool_name,
        "error": error,
        "receivedArgs": received_args,
        "source": "storage-migration",
    })
}

fn sanitize_tool_call_input_for_storage(tool_name: &str, input: &Value) -> (Value, bool) {
    if input.is_object() {
        return (input.clone(), false);
    }

    if let Some(raw) = input.as_str() {
        return (
            invalid_tool_payload_for_storage(
                tool_name,
                "Stored tool arguments are malformed/truncated and cannot be replayed safely.",
                serde_json::json!({
                    "type": "string",
                    "raw_len": raw.len(),
                    "preview": raw.chars().take(240).collect::<String>(),
                }),
            ),
            true,
        );
    }

    let input_type = match input {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::String(_) => "string",
    };
    (
        invalid_tool_payload_for_storage(
            tool_name,
            "Stored tool arguments are non-object and cannot be replayed safely.",
            serde_json::json!({
                "type": input_type,
            }),
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::sanitize_tool_call_input_for_storage;

    #[tokio::test]
    async fn migrations_upgrade_legacy_db_without_signature_column() {
        // 复现 2026-07 启动卡死的场景：老库的 memory_records 没有 signature 列。
        // signature 索引在 CREATE_INDEXES 批里，若 ALTER 排在它之后，老库升级
        // 时索引创建引用不存在的列（sqlx 多语句批下表现为挂起而非报错）。
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE memory_records (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, scope TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'candidate',
                title TEXT NOT NULL, summary TEXT NOT NULL,
                trigger_conditions TEXT, normalized_facts TEXT,
                boundaries TEXT, confidence REAL,
                source_session_id TEXT, workspace_identity TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                last_validated_at INTEGER, expires_at INTEGER,
                derived_skill_name TEXT, linked_skill_name TEXT,
                validation_status TEXT NOT NULL DEFAULT 'pending'
            )"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("PRAGMA user_version = 1")
            .execute(&pool)
            .await
            .unwrap();

        let db = super::Database { pool };
        db.run_migrations()
            .await
            .expect("migrations should succeed");

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('memory_records')")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert!(columns.iter().any(|c| c == "signature"));
        let index: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_memory_records_signature'",
        )
        .fetch_optional(db.pool())
        .await
        .unwrap();
        assert!(index.is_some());
        let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert!(user_version >= 2);
    }

    #[tokio::test]
    async fn memory_signature_backfill_fills_null_rows_and_is_idempotent() {
        let db = crate::Database::in_memory()
            .await
            .expect("db should initialize");

        // Fresh databases run the v2 migration gate during initialization.
        let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert!(user_version >= 2);

        // Simulate a row written before the signature column existed.
        sqlx::query(
            r#"INSERT INTO memory_records (
                id, kind, scope, status, title, summary,
                trigger_conditions, normalized_facts,
                created_at, updated_at, validation_status, signature
            ) VALUES (
                'mem_legacy', 'pattern', 'workspace_shared', 'candidate',
                'Legacy title', 'Legacy summary for backfill',
                '["tool:x"]', '["a=1"]',
                1, 2, 'pending', NULL
            )"#,
        )
        .execute(db.pool())
        .await
        .unwrap();

        db.backfill_memory_record_signatures().await.unwrap();

        let signature: Option<String> =
            sqlx::query_scalar("SELECT signature FROM memory_records WHERE id = 'mem_legacy'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        let expected = crate::repository::memory_record_signature_parts(
            "workspace_shared",
            "Legacy title",
            "Legacy summary for backfill",
            &["tool:x".to_string()],
            &["a=1".to_string()],
        );
        assert_eq!(signature.as_deref(), Some(expected.as_str()));

        // Idempotent: rows that already carry a signature are left untouched.
        sqlx::query("UPDATE memory_records SET signature = 'sentinel' WHERE id = 'mem_legacy'")
            .execute(db.pool())
            .await
            .unwrap();
        db.backfill_memory_record_signatures().await.unwrap();
        let signature: Option<String> =
            sqlx::query_scalar("SELECT signature FROM memory_records WHERE id = 'mem_legacy'")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(signature.as_deref(), Some("sentinel"));
    }

    #[test]
    fn sanitize_tool_call_input_for_storage_rejects_jsonish() {
        let raw = serde_json::Value::String(
            "{\"file_path\":\"t2.html\",\"content\":\"<!DOCTYPE html>".to_string(),
        );
        let (sanitized, rerouted_invalid) = sanitize_tool_call_input_for_storage("write", &raw);
        assert!(sanitized.is_object());
        assert!(rerouted_invalid);
        assert_eq!(sanitized["tool"], "write");
    }

    #[test]
    fn sanitize_tool_call_input_for_storage_routes_unrecoverable_to_invalid_payload() {
        let raw = serde_json::Value::String("not-json".to_string());
        let (sanitized, rerouted_invalid) = sanitize_tool_call_input_for_storage("write", &raw);
        assert!(sanitized.is_object());
        assert!(rerouted_invalid);
        assert_eq!(sanitized["tool"], "write");
        assert_eq!(sanitized["receivedArgs"]["type"], "string");
        assert!(sanitized["error"]
            .as_str()
            .unwrap_or_default()
            .contains("malformed/truncated"));
    }

    #[test]
    fn sanitize_tool_call_input_for_storage_keeps_object_without_sentinel_semantics() {
        let raw = serde_json::json!({
            "_agendao_unrecoverable_tool_args": true,
            "raw_len": 42,
            "raw_preview": "{\"content\":\"<html>"
        });
        let (sanitized, rerouted_invalid) = sanitize_tool_call_input_for_storage("write", &raw);
        assert!(sanitized.is_object());
        assert!(!rerouted_invalid);
        assert_eq!(sanitized, raw);
    }
}
