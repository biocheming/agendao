//! Integration tests for `agendao-storage-sqlite`.
//!
//! These tests exercise the crate's public API against real SQLite databases
//! (both `sqlite::memory:` and tempfile-backed files): base migrations leave a
//! usable schema, the schema supports full insert/read/update/delete
//! roundtrips for the session and message tables, and data survives a
//! reconnect (process restart simulation).
//!
//! Note: the repository trait implementations (`SqliteSessionRepository`,
//! `SqliteMessageRepository`, `SqliteArtifactRepository`) currently return
//! `StorageError::Unimplemented` for every trait method; the roundtrips below
//! therefore drive the schema through raw SQL on the pools the repositories
//! expose. `repositories_report_unimplemented` documents that status and acts
//! as a tripwire once the real implementations land.

use agendao_storage_core::{
    ArtifactRecord, ArtifactRepository, MessageRecord, MessageRepository, SessionRecord,
    SessionRepository, StorageBackend, StorageError,
};
use agendao_storage_sqlite::{
    SqliteConfig, SqliteStorage, SQLITE_BASE_MIGRATIONS,
};
use sqlx::Row;

async fn connect_memory() -> SqliteStorage {
    SqliteStorage::connect(&SqliteConfig::default())
        .await
        .expect("connect to in-memory sqlite")
}

async fn run_migrations(storage: &SqliteStorage) {
    for migration in SQLITE_BASE_MIGRATIONS {
        sqlx::query(migration)
            .execute(storage.pool())
            .await
            .expect("base migration applies cleanly");
    }
}

async fn connect_migrated_memory() -> SqliteStorage {
    let storage = connect_memory().await;
    run_migrations(&storage).await;
    storage
}

async fn table_columns(storage: &SqliteStorage, table: &str) -> Vec<String> {
    sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(storage.pool())
        .await
        .expect("table_info query succeeds")
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect()
}

#[tokio::test]
async fn default_config_uses_in_memory_sqlite() {
    let config = SqliteConfig::default();
    assert_eq!(config.url, "sqlite::memory:");
    assert_eq!(config.max_connections, 1);
}

#[tokio::test]
async fn healthcheck_succeeds_after_connect() {
    let storage = connect_memory().await;
    storage.healthcheck().await.expect("healthcheck succeeds");
}

#[tokio::test]
async fn connect_rejects_invalid_url() {
    let config = SqliteConfig {
        url: "not-a-sqlite-url".to_string(),
        max_connections: 1,
    };
    let result = SqliteStorage::connect(&config).await;
    match result {
        Err(StorageError::Backend(_)) => {}
        Err(other) => panic!("expected backend error for invalid url, got {other:?}"),
        Ok(_) => panic!("expected backend error for invalid url, got Ok(storage)"),
    }
}

#[tokio::test]
async fn migrations_create_expected_schema() {
    let storage = connect_migrated_memory().await;

    for table in ["sessions", "messages", "artifacts"] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(storage.pool())
        .await
        .expect("sqlite_master query succeeds");
        assert_eq!(exists, 1, "table {table} should exist after migrations");
    }

    assert_eq!(
        table_columns(&storage, "sessions").await,
        vec!["id", "title", "directory", "created_at_ms", "updated_at_ms"]
    );
    assert_eq!(
        table_columns(&storage, "messages").await,
        vec!["id", "session_id", "role", "content", "created_at_ms"]
    );
    assert_eq!(
        table_columns(&storage, "artifacts").await,
        vec!["id", "session_id", "path", "kind", "created_at_ms"]
    );
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let storage = connect_migrated_memory().await;
    run_migrations(&storage).await;
    storage.healthcheck().await.expect("still healthy after re-run");
}

async fn insert_session(storage: &SqliteStorage, session: &SessionRecord) {
    sqlx::query(
        "INSERT INTO sessions (id, title, directory, created_at_ms, updated_at_ms) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&session.id)
    .bind(&session.title)
    .bind(&session.directory)
    .bind(session.created_at_ms)
    .bind(session.updated_at_ms)
    .execute(storage.sessions().pool())
    .await
    .expect("session insert succeeds");
}

async fn read_session(storage: &SqliteStorage, id: &str) -> Option<SessionRecord> {
    sqlx::query(
        "SELECT id, title, directory, created_at_ms, updated_at_ms FROM sessions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(storage.sessions().pool())
    .await
    .expect("session select succeeds")
    .map(|row| SessionRecord {
        id: row.get("id"),
        title: row.get("title"),
        directory: row.get("directory"),
        created_at_ms: row.get("created_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
    })
}

fn sample_session(id: &str, updated_at_ms: i64) -> SessionRecord {
    SessionRecord {
        id: id.to_string(),
        title: format!("session {id}"),
        directory: "/tmp/project".to_string(),
        created_at_ms: 1_000,
        updated_at_ms,
    }
}

fn sample_message(id: &str, session_id: &str, created_at_ms: i64) -> MessageRecord {
    MessageRecord {
        id: id.to_string(),
        session_id: session_id.to_string(),
        role: "user".to_string(),
        content: format!("message {id}"),
        created_at_ms,
    }
}

async fn insert_message(storage: &SqliteStorage, message: &MessageRecord) {
    sqlx::query(
        "INSERT INTO messages (id, session_id, role, content, created_at_ms) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&message.id)
    .bind(&message.session_id)
    .bind(&message.role)
    .bind(&message.content)
    .bind(message.created_at_ms)
    .execute(storage.messages().pool())
    .await
    .expect("message insert succeeds");
}

async fn list_messages(storage: &SqliteStorage, session_id: &str) -> Vec<MessageRecord> {
    sqlx::query(
        "SELECT id, session_id, role, content, created_at_ms FROM messages \
         WHERE session_id = ? ORDER BY created_at_ms, id",
    )
    .bind(session_id)
    .fetch_all(storage.messages().pool())
    .await
    .expect("message select succeeds")
    .iter()
    .map(|row| MessageRecord {
        id: row.get("id"),
        session_id: row.get("session_id"),
        role: row.get("role"),
        content: row.get("content"),
        created_at_ms: row.get("created_at_ms"),
    })
    .collect()
}

#[tokio::test]
async fn session_schema_supports_insert_read_update_delete_roundtrip() {
    let storage = connect_migrated_memory().await;
    let session = sample_session("s1", 2_000);

    insert_session(&storage, &session).await;
    assert_eq!(read_session(&storage, "s1").await, Some(session.clone()));

    sqlx::query("UPDATE sessions SET title = ?, updated_at_ms = ? WHERE id = ?")
        .bind("renamed")
        .bind(3_000_i64)
        .bind("s1")
        .execute(storage.sessions().pool())
        .await
        .expect("session update succeeds");
    let updated = read_session(&storage, "s1")
        .await
        .expect("session still present after update");
    assert_eq!(updated.title, "renamed");
    assert_eq!(updated.updated_at_ms, 3_000);
    assert_eq!(updated.created_at_ms, session.created_at_ms);

    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind("s1")
        .execute(storage.sessions().pool())
        .await
        .expect("session delete succeeds");
    assert_eq!(read_session(&storage, "s1").await, None);
}

#[tokio::test]
async fn message_schema_supports_append_list_and_delete_roundtrip() {
    let storage = connect_migrated_memory().await;
    insert_session(&storage, &sample_session("s1", 2_000)).await;

    let first = sample_message("m1", "s1", 1_100);
    let second = MessageRecord {
        role: "assistant".to_string(),
        ..sample_message("m2", "s1", 1_200)
    };
    let other_session = sample_message("m3", "s2", 1_150);
    insert_message(&storage, &first).await;
    insert_message(&storage, &second).await;
    insert_message(&storage, &other_session).await;

    assert_eq!(list_messages(&storage, "s1").await, vec![first, second]);
    assert_eq!(list_messages(&storage, "s2").await, vec![other_session]);

    sqlx::query("DELETE FROM messages WHERE session_id = ?")
        .bind("s1")
        .execute(storage.messages().pool())
        .await
        .expect("message delete succeeds");
    assert!(list_messages(&storage, "s1").await.is_empty());
    assert_eq!(list_messages(&storage, "s2").await.len(), 1);
}

#[tokio::test]
async fn artifact_schema_supports_insert_and_list_roundtrip() {
    let storage = connect_migrated_memory().await;
    let artifact = ArtifactRecord {
        id: "a1".to_string(),
        session_id: "s1".to_string(),
        path: "out/report.md".to_string(),
        kind: "file".to_string(),
        created_at_ms: 1_500,
    };

    sqlx::query(
        "INSERT INTO artifacts (id, session_id, path, kind, created_at_ms) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&artifact.id)
    .bind(&artifact.session_id)
    .bind(&artifact.path)
    .bind(&artifact.kind)
    .bind(artifact.created_at_ms)
    .execute(storage.artifacts().pool())
    .await
    .expect("artifact insert succeeds");

    let rows = sqlx::query(
        "SELECT id, session_id, path, kind, created_at_ms FROM artifacts WHERE session_id = ?",
    )
    .bind("s1")
    .fetch_all(storage.artifacts().pool())
    .await
    .expect("artifact select succeeds");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.get::<String, _>("id"), artifact.id);
    assert_eq!(row.get::<String, _>("path"), artifact.path);
    assert_eq!(row.get::<String, _>("kind"), artifact.kind);
    assert_eq!(row.get::<i64, _>("created_at_ms"), artifact.created_at_ms);
}

#[tokio::test]
async fn data_persists_across_reconnect() {
    let tempdir = tempfile::tempdir().expect("create tempdir");
    let db_path = tempdir.path().join("agendao-storage-sqlite-test.db");
    let config = SqliteConfig {
        url: format!("sqlite:{}?mode=rwc", db_path.display()),
        max_connections: 1,
    };

    let session = sample_session("s-persist", 2_000);
    let message = sample_message("m-persist", "s-persist", 1_100);
    {
        let storage = SqliteStorage::connect(&config)
            .await
            .expect("connect to file-backed sqlite");
        run_migrations(&storage).await;
        insert_session(&storage, &session).await;
        insert_message(&storage, &message).await;
        storage.pool().close().await;
    }

    let reopened = SqliteStorage::connect(&config)
        .await
        .expect("reconnect to file-backed sqlite");
    reopened
        .healthcheck()
        .await
        .expect("healthcheck after reconnect");
    assert_eq!(read_session(&reopened, "s-persist").await, Some(session));
    assert_eq!(list_messages(&reopened, "s-persist").await, vec![message]);
    reopened.pool().close().await;
}

#[tokio::test]
async fn backend_accessors_share_one_pool() {
    let storage = connect_migrated_memory().await;
    insert_session(&storage, &sample_session("s-shared", 2_000)).await;
    // Written through the sessions repository's pool, visible through the
    // top-level pool and the other repositories' pools.
    assert!(read_session(&storage, "s-shared").await.is_some());
    let visible_via_messages: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = 's-shared'")
            .fetch_one(storage.messages().pool())
            .await
            .expect("select via messages pool succeeds");
    assert_eq!(visible_via_messages, 1);
}

/// Characterization of the current state: every repository trait method is a
/// stub returning `StorageError::Unimplemented`. Once the real SQL
/// implementations land, this test should be replaced with trait-level
/// roundtrip tests.
#[tokio::test]
async fn repositories_report_unimplemented() {
    let storage = connect_memory().await;
    let session = sample_session("s1", 2_000);
    let message = sample_message("m1", "s1", 1_100);
    let artifact = ArtifactRecord {
        id: "a1".to_string(),
        session_id: "s1".to_string(),
        path: "out.txt".to_string(),
        kind: "file".to_string(),
        created_at_ms: 1_500,
    };

    assert!(matches!(
        storage.sessions().get("s1").await,
        Err(StorageError::Unimplemented(_))
    ));
    assert!(matches!(
        storage.sessions().upsert(&session).await,
        Err(StorageError::Unimplemented(_))
    ));
    assert!(matches!(
        storage.sessions().list_recent(10).await,
        Err(StorageError::Unimplemented(_))
    ));
    assert!(matches!(
        storage.messages().list_for_session("s1").await,
        Err(StorageError::Unimplemented(_))
    ));
    assert!(matches!(
        storage.messages().append(&message).await,
        Err(StorageError::Unimplemented(_))
    ));
    assert!(matches!(
        storage.artifacts().list_for_session("s1").await,
        Err(StorageError::Unimplemented(_))
    ));
    assert!(matches!(
        storage.artifacts().insert(&artifact).await,
        Err(StorageError::Unimplemented(_))
    ));
}
