use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

static SQLITE_VEC_REGISTRATION: OnceLock<()> = OnceLock::new();
static RAG_POOLS: OnceLock<Mutex<HashMap<String, SqlitePool>>> = OnceLock::new();

fn register_sqlite_vec_extension() {
    SQLITE_VEC_REGISTRATION.get_or_init(|| unsafe {
        libsqlite3_sys::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

fn rag_db_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("rag")
        .join("rag.sqlite3");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

pub async fn get_pool(app: &AppHandle) -> Result<SqlitePool, String> {
    register_sqlite_vec_extension();

    let path = rag_db_path(app)?;
    let key = path.display().to_string();
    let pools = RAG_POOLS.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = pools.lock().await;
        if let Some(pool) = guard.get(&key) {
            return Ok(pool.clone());
        }
    }

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|e| e.to_string())?;
    ensure_schema(&pool).await?;

    let mut guard = pools.lock().await;
    Ok(guard.entry(key).or_insert_with(|| pool.clone()).clone())
}

async fn ensure_schema(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS rag_documents (
            id TEXT PRIMARY KEY,
            scope TEXT NOT NULL,
            source_name TEXT NOT NULL,
            source_type TEXT NOT NULL,
            mime_type TEXT,
            content TEXT NOT NULL,
            content_chars INTEGER NOT NULL,
            checksum TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(scope, source_name)
        );

        CREATE TABLE IF NOT EXISTS rag_chunks (
            id TEXT PRIMARY KEY,
            document_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            content TEXT NOT NULL,
            content_chars INTEGER NOT NULL,
            checksum TEXT NOT NULL,
            embedding_model TEXT NOT NULL,
            embedding BLOB NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY(document_id) REFERENCES rag_documents(id) ON DELETE CASCADE,
            UNIQUE(document_id, chunk_index)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunks_fts USING fts5(
            chunk_id UNINDEXED,
            document_id UNINDEXED,
            scope UNINDEXED,
            source_name,
            content,
            tokenize='unicode61'
        );

        CREATE INDEX IF NOT EXISTS idx_rag_documents_scope_updated
            ON rag_documents(scope, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_rag_documents_scope_source
            ON rag_documents(scope, source_name);
        CREATE INDEX IF NOT EXISTS idx_rag_chunks_scope_model
            ON rag_chunks(scope, embedding_model);
        CREATE INDEX IF NOT EXISTS idx_rag_chunks_document
            ON rag_chunks(document_id, chunk_index);
        "#,
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}
