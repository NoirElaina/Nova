mod chunking;
mod db;
mod embeddings;
pub mod types;

use self::chunking::{
    normalize_content, normalize_optional_string, normalize_source_name, normalize_source_type,
    preview_text, split_into_chunks, MAX_BATCH_SIZE, MAX_DOCUMENT_BYTES,
};
use self::embeddings::{embed_texts, embedding_to_blob, load_embedding_config};
pub use self::types::{
    RagDocumentContent, RagDocumentInput, RagDocumentMeta, RagRejectedItem, RagSearchHit, RagStats,
    RagUpsertResult,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use tauri::AppHandle;
use tracing::{debug, warn};
use uuid::Uuid;

const GLOBAL_SCOPE: &str = "global";
const CONVERSATION_SCOPE_PREFIX: &str = "conversation:";
const SEARCH_LIMIT_DEFAULT: usize = 5;
const SEARCH_LIMIT_MAX: usize = 50;
const SEARCH_CANDIDATE_MULTIPLIER: usize = 4;
const VECTOR_WEIGHT: f64 = 0.70;
const FTS_WEIGHT: f64 = 0.30;
const FTS_ONLY_EMBEDDING_MODEL: &str = "__fts_only__";

struct PreparedDocument {
    source_name: String,
    source_type: String,
    mime_type: Option<String>,
    content: String,
    content_chars: usize,
    checksum: String,
    chunks: Vec<String>,
}

#[derive(Default, Clone)]
struct CandidateScore {
    vector_score: Option<f64>,
    fts_score: Option<f64>,
}

struct HitDetail {
    document_id: String,
    source_name: String,
    source_type: String,
    mime_type: Option<String>,
    snippet: String,
    document_chars: usize,
    updated_at: i64,
}

fn scope_from_conversation_id(conversation_id: Option<&str>) -> Result<String, String> {
    match conversation_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => Ok(format!("{}{}", CONVERSATION_SCOPE_PREFIX, id)),
        None => Ok(GLOBAL_SCOPE.to_string()),
    }
}

fn required_conversation_scope(conversation_id: &str) -> Result<String, String> {
    let id = conversation_id.trim();
    if id.is_empty() {
        return Err("conversation_id is required".to_string());
    }
    Ok(format!("{}{}", CONVERSATION_SCOPE_PREFIX, id))
}

fn checksum_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_ts() -> i64 {
    Utc::now().timestamp()
}

fn i64_to_usize(value: i64) -> usize {
    usize::try_from(value.max(0)).unwrap_or(usize::MAX)
}

async fn scope_stats(pool: &SqlitePool, scope: &str) -> Result<RagStats, String> {
    let row = sqlx::query(
        r#"
        SELECT
            COUNT(*) AS document_count,
            COALESCE(SUM(content_chars), 0) AS total_chars,
            MAX(updated_at) AS last_updated_at
        FROM rag_documents
        WHERE scope = ?
        "#,
    )
    .bind(scope)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(RagStats {
        document_count: i64_to_usize(row.get::<i64, _>("document_count")),
        total_chars: i64_to_usize(row.get::<i64, _>("total_chars")),
        last_updated_at: row.get::<Option<i64>, _>("last_updated_at"),
    })
}

pub async fn get_stats(app: AppHandle) -> Result<RagStats, String> {
    let pool = db::get_pool(&app).await?;
    scope_stats(&pool, GLOBAL_SCOPE).await
}

pub async fn list_documents(app: AppHandle) -> Result<Vec<RagDocumentMeta>, String> {
    let pool = db::get_pool(&app).await?;
    list_documents_for_scope(&pool, GLOBAL_SCOPE).await
}

pub async fn list_conversation_documents(
    app: AppHandle,
    conversation_id: String,
) -> Result<Vec<RagDocumentMeta>, String> {
    let pool = db::get_pool(&app).await?;
    let scope = required_conversation_scope(&conversation_id)?;
    list_documents_for_scope(&pool, &scope).await
}

async fn list_documents_for_scope(
    pool: &SqlitePool,
    scope: &str,
) -> Result<Vec<RagDocumentMeta>, String> {
    let rows = sqlx::query(
        r#"
        SELECT id, source_name, source_type, mime_type, content, content_chars, checksum, created_at, updated_at
        FROM rag_documents
        WHERE scope = ?
        ORDER BY updated_at DESC, source_name ASC
        "#,
    )
    .bind(scope)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let content = row.get::<String, _>("content");
            RagDocumentMeta {
                id: row.get("id"),
                source_name: row.get("source_name"),
                source_type: row.get("source_type"),
                mime_type: row.get("mime_type"),
                content_chars: i64_to_usize(row.get::<i64, _>("content_chars")),
                preview: preview_text(&content),
                checksum: row.get("checksum"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            }
        })
        .collect())
}

pub async fn read_document(
    app: AppHandle,
    document_id: String,
    conversation_id: Option<String>,
) -> Result<Option<RagDocumentContent>, String> {
    let id = document_id.trim();
    if id.is_empty() {
        return Err("document_id is required".to_string());
    }

    let pool = db::get_pool(&app).await?;
    let scope = scope_from_conversation_id(conversation_id.as_deref())?;
    let row = sqlx::query(
        r#"
        SELECT id, source_name, source_type, mime_type, content, content_chars, checksum, created_at, updated_at
        FROM rag_documents
        WHERE id = ? AND scope = ?
        "#,
    )
    .bind(id)
    .bind(scope)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.map(|row| RagDocumentContent {
        id: row.get("id"),
        source_name: row.get("source_name"),
        source_type: row.get("source_type"),
        mime_type: row.get("mime_type"),
        content: row.get("content"),
        content_chars: i64_to_usize(row.get::<i64, _>("content_chars")),
        checksum: row.get("checksum"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }))
}

pub async fn upsert_documents(
    app: AppHandle,
    documents: Vec<RagDocumentInput>,
) -> Result<RagUpsertResult, String> {
    upsert_documents_for_scope(app, None, documents).await
}

pub async fn upsert_conversation_documents(
    app: AppHandle,
    conversation_id: String,
    documents: Vec<RagDocumentInput>,
) -> Result<RagUpsertResult, String> {
    let normalized_conversation_id = normalize_optional_string(Some(conversation_id))
        .ok_or_else(|| "conversation_id is required".to_string())?;
    upsert_documents_for_scope(app, Some(normalized_conversation_id), documents).await
}

async fn upsert_documents_for_scope(
    app: AppHandle,
    forced_conversation_id: Option<String>,
    documents: Vec<RagDocumentInput>,
) -> Result<RagUpsertResult, String> {
    if documents.is_empty() {
        return Err("No documents provided".to_string());
    }
    if documents.len() > MAX_BATCH_SIZE {
        return Err(format!(
            "Batch size exceeded: max {} documents per request",
            MAX_BATCH_SIZE
        ));
    }

    let pool = db::get_pool(&app).await?;
    let mut prepared = Vec::new();
    let mut rejected = Vec::new();

    for (index, item) in documents.into_iter().enumerate() {
        let source_name = normalize_source_name(&item.source_name, index);
        let content = normalize_content(&item.content);
        if content.is_empty() {
            rejected.push(RagRejectedItem {
                source_name,
                reason: "内容为空".to_string(),
            });
            continue;
        }
        if content.as_bytes().len() > MAX_DOCUMENT_BYTES {
            rejected.push(RagRejectedItem {
                source_name,
                reason: format!("文件超过后端上限 {}KB", MAX_DOCUMENT_BYTES / 1024),
            });
            continue;
        }

        let content_chars = content.chars().count();
        let chunks = split_into_chunks(&content);
        prepared.push(PreparedDocument {
            source_name,
            source_type: normalize_source_type(&item.source_type),
            mime_type: normalize_optional_string(item.mime_type),
            checksum: checksum_hex(&content),
            content,
            content_chars,
            chunks,
        });
    }

    let scope = if let Some(scope_id) = forced_conversation_id.as_deref() {
        required_conversation_scope(scope_id)?
    } else {
        GLOBAL_SCOPE.to_string()
    };

    if prepared.is_empty() {
        let stats = scope_stats(&pool, &scope).await?;
        return Ok(RagUpsertResult {
            added: 0,
            updated: 0,
            rejected,
            total_documents: stats.document_count,
            total_chars: stats.total_chars,
        });
    }

    let chunk_texts = prepared
        .iter()
        .flat_map(|doc| doc.chunks.iter().cloned())
        .collect::<Vec<_>>();
    let embedding_plan = match load_embedding_config(&app) {
        Ok(config) => match embed_texts(&config, &chunk_texts).await {
            Ok(embeddings) if embeddings.len() == chunk_texts.len() => {
                Some((config.model.clone(), embeddings))
            }
            Ok(embeddings) => {
                warn!(
                    expected = chunk_texts.len(),
                    actual = embeddings.len(),
                    "RAG embedding count mismatch; falling back to FTS-only indexing"
                );
                None
            }
            Err(error) => {
                warn!(error = %error, "RAG embedding failed; falling back to FTS-only indexing");
                None
            }
        },
        Err(error) => {
            debug!(error = %error, "RAG embedding unavailable; falling back to FTS-only indexing");
            None
        }
    };

    let mut embedding_index = 0usize;
    let now = now_ts();
    let mut added = 0u32;
    let mut updated = 0u32;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    for document in prepared {
        let existing = sqlx::query(
            "SELECT id, created_at FROM rag_documents WHERE scope = ? AND source_name = ?",
        )
        .bind(&scope)
        .bind(&document.source_name)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let (document_id, created_at, is_update) = if let Some(row) = existing {
            (
                row.get::<String, _>("id"),
                row.get::<i64, _>("created_at"),
                true,
            )
        } else {
            (Uuid::new_v4().to_string(), now, false)
        };

        if is_update {
            sqlx::query("DELETE FROM rag_chunks_fts WHERE document_id = ?")
                .bind(&document_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            sqlx::query("DELETE FROM rag_chunks WHERE document_id = ?")
                .bind(&document_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            updated += 1;
        } else {
            added += 1;
        }

        sqlx::query(
            r#"
            INSERT INTO rag_documents
                (id, scope, source_name, source_type, mime_type, content, content_chars, checksum, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                source_name = excluded.source_name,
                source_type = excluded.source_type,
                mime_type = excluded.mime_type,
                content = excluded.content,
                content_chars = excluded.content_chars,
                checksum = excluded.checksum,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&document_id)
        .bind(&scope)
        .bind(&document.source_name)
        .bind(&document.source_type)
        .bind(&document.mime_type)
        .bind(&document.content)
        .bind(document.content_chars as i64)
        .bind(&document.checksum)
        .bind(created_at)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        for (chunk_index, chunk) in document.chunks.iter().enumerate() {
            let (embedding_model, embedding_blob) =
                if let Some((model, embeddings)) = embedding_plan.as_ref() {
                    let embedding = embeddings
                        .get(embedding_index)
                        .ok_or_else(|| "Embedding result count mismatch".to_string())?;
                    embedding_index += 1;
                    (model.as_str(), embedding_to_blob(embedding))
                } else {
                    (FTS_ONLY_EMBEDDING_MODEL, Vec::new())
                };
            let chunk_id = Uuid::new_v4().to_string();
            let chunk_checksum = checksum_hex(chunk);

            sqlx::query(
                r#"
                INSERT INTO rag_chunks
                    (id, document_id, scope, chunk_index, content, content_chars, checksum, embedding_model, embedding, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&chunk_id)
            .bind(&document_id)
            .bind(&scope)
            .bind(chunk_index as i64)
            .bind(chunk)
            .bind(chunk.chars().count() as i64)
            .bind(chunk_checksum)
            .bind(embedding_model)
            .bind(embedding_blob)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

            sqlx::query(
                r#"
                INSERT INTO rag_chunks_fts (chunk_id, document_id, scope, source_name, content)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&chunk_id)
            .bind(&document_id)
            .bind(&scope)
            .bind(&document.source_name)
            .bind(chunk)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    let stats = scope_stats(&pool, &scope).await?;
    Ok(RagUpsertResult {
        added,
        updated,
        rejected,
        total_documents: stats.document_count,
        total_chars: stats.total_chars,
    })
}

pub async fn search_documents(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<RagSearchHit>, String> {
    search_documents_for_scope(app, query, limit, GLOBAL_SCOPE.to_string()).await
}

pub async fn search_conversation_documents(
    app: AppHandle,
    conversation_id: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<RagSearchHit>, String> {
    let scope = required_conversation_scope(&conversation_id)?;
    search_documents_for_scope(app, query, limit, scope).await
}

async fn search_documents_for_scope(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
    scope: String,
) -> Result<Vec<RagSearchHit>, String> {
    let query_text = query.trim();
    if query_text.is_empty() {
        return Err("query is required".to_string());
    }

    let max_hits = limit
        .unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(1, SEARCH_LIMIT_MAX);
    let pool = db::get_pool(&app).await?;
    let doc_count = sqlx::query("SELECT COUNT(*) AS count FROM rag_documents WHERE scope = ?")
        .bind(&scope)
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .get::<i64, _>("count");
    if doc_count <= 0 {
        return Ok(Vec::new());
    }

    let candidate_limit = max_hits * SEARCH_CANDIDATE_MULTIPLIER;
    let mut candidates: HashMap<String, CandidateScore> = HashMap::new();

    match load_embedding_config(&app) {
        Ok(config) => match embed_texts(&config, &[query_text.to_string()]).await {
            Ok(query_embeddings) => {
                if let Some(query_embedding) = query_embeddings.first() {
                    collect_vector_candidates(
                        &pool,
                        &scope,
                        &config.model,
                        &embedding_to_blob(query_embedding),
                        candidate_limit,
                        &mut candidates,
                    )
                    .await?;
                }
            }
            Err(error) => {
                warn!(error = %error, "RAG query embedding failed; using FTS-only search");
            }
        },
        Err(error) => {
            debug!(error = %error, "RAG query embedding unavailable; using FTS-only search");
        }
    }

    if let Some(fts_query) = build_fts_query(query_text) {
        collect_fts_candidates(&pool, &scope, &fts_query, candidate_limit, &mut candidates).await?;
    }

    let mut ranked = candidates
        .into_iter()
        .map(|(chunk_id, score)| (chunk_id, final_score(&score)))
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

    let mut hits = Vec::new();
    let mut seen_documents = HashSet::new();
    for (chunk_id, score) in ranked {
        if hits.len() >= max_hits {
            break;
        }
        let Some(detail) = load_hit_detail(&pool, &chunk_id).await? else {
            continue;
        };
        if !seen_documents.insert(detail.document_id.clone()) {
            continue;
        }
        hits.push(RagSearchHit {
            id: detail.document_id,
            source_name: detail.source_name,
            source_type: detail.source_type,
            mime_type: detail.mime_type,
            score: (score * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32,
            snippet: detail.snippet,
            content_chars: detail.document_chars,
            updated_at: detail.updated_at,
        });
    }

    Ok(hits)
}

async fn collect_vector_candidates(
    pool: &SqlitePool,
    scope: &str,
    embedding_model: &str,
    query_embedding_blob: &[u8],
    limit: usize,
    candidates: &mut HashMap<String, CandidateScore>,
) -> Result<(), String> {
    let rows = sqlx::query(
        r#"
        SELECT id AS chunk_id, vec_distance_cosine(embedding, ?) AS distance
        FROM rag_chunks
        WHERE scope = ? AND embedding_model = ?
        ORDER BY distance ASC
        LIMIT ?
        "#,
    )
    .bind(query_embedding_blob)
    .bind(scope)
    .bind(embedding_model)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for row in rows {
        let chunk_id = row.get::<String, _>("chunk_id");
        let distance = row.get::<Option<f64>, _>("distance").unwrap_or(1.0);
        let score = 1.0 / (1.0 + distance.max(0.0));
        candidates.entry(chunk_id).or_default().vector_score = Some(score);
    }
    Ok(())
}

async fn collect_fts_candidates(
    pool: &SqlitePool,
    scope: &str,
    fts_query: &str,
    limit: usize,
    candidates: &mut HashMap<String, CandidateScore>,
) -> Result<(), String> {
    let rows = sqlx::query(
        r#"
        SELECT chunk_id, bm25(rag_chunks_fts) AS rank
        FROM rag_chunks_fts
        WHERE rag_chunks_fts MATCH ? AND scope = ?
        ORDER BY bm25(rag_chunks_fts)
        LIMIT ?
        "#,
    )
    .bind(fts_query)
    .bind(scope)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for row in rows {
        let chunk_id = row.get::<String, _>("chunk_id");
        let rank = row.get::<f64, _>("rank");
        let score = 1.0 / (1.0 + rank.abs());
        candidates.entry(chunk_id).or_default().fts_score = Some(score);
    }
    Ok(())
}

async fn load_hit_detail(pool: &SqlitePool, chunk_id: &str) -> Result<Option<HitDetail>, String> {
    let row = sqlx::query(
        r#"
        SELECT
            c.id AS chunk_id,
            c.content AS chunk_content,
            d.id AS document_id,
            d.source_name AS source_name,
            d.source_type AS source_type,
            d.mime_type AS mime_type,
            d.content_chars AS document_chars,
            d.updated_at AS updated_at
        FROM rag_chunks c
        JOIN rag_documents d ON d.id = c.document_id
        WHERE c.id = ?
        "#,
    )
    .bind(chunk_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.map(|row| HitDetail {
        document_id: row.get("document_id"),
        source_name: row.get("source_name"),
        source_type: row.get("source_type"),
        mime_type: row.get("mime_type"),
        snippet: row.get("chunk_content"),
        document_chars: i64_to_usize(row.get::<i64, _>("document_chars")),
        updated_at: row.get("updated_at"),
    }))
}

fn final_score(score: &CandidateScore) -> f64 {
    match (score.vector_score, score.fts_score) {
        (Some(vector), Some(fts)) => vector * VECTOR_WEIGHT + fts * FTS_WEIGHT,
        (Some(vector), None) => vector,
        (None, Some(fts)) => fts,
        (None, None) => 0.0,
    }
}

fn build_fts_query(query: &str) -> Option<String> {
    let terms = query
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .take(12)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

pub async fn remove_document(app: AppHandle, document_id: String) -> Result<bool, String> {
    let id = document_id.trim();
    if id.is_empty() {
        return Err("document_id is required".to_string());
    }

    let pool = db::get_pool(&app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        r#"
        DELETE FROM rag_chunks_fts
        WHERE document_id IN (SELECT id FROM rag_documents WHERE id = ? AND scope = ?)
        "#,
    )
    .bind(id)
    .bind(GLOBAL_SCOPE)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let result = sqlx::query("DELETE FROM rag_documents WHERE id = ? AND scope = ?")
        .bind(id)
        .bind(GLOBAL_SCOPE)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

pub async fn clear_documents(app: AppHandle) -> Result<(), String> {
    let pool = db::get_pool(&app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        r#"
        DELETE FROM rag_chunks_fts
        WHERE document_id IN (SELECT id FROM rag_documents WHERE scope = ?)
        "#,
    )
    .bind(GLOBAL_SCOPE)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM rag_documents WHERE scope = ?")
        .bind(GLOBAL_SCOPE)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn remove_conversation_documents(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<usize, String> {
    let scope = required_conversation_scope(conversation_id)?;
    remove_scope_documents(app, &scope).await
}

pub async fn remove_all_conversation_documents(app: &AppHandle) -> Result<usize, String> {
    let pool = db::get_pool(app).await?;
    let document_ids = sqlx::query("SELECT id FROM rag_documents WHERE scope LIKE ?")
        .bind(format!("{}%", CONVERSATION_SCOPE_PREFIX))
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|row| row.get::<String, _>("id"))
        .collect::<Vec<_>>();

    if document_ids.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for document_id in &document_ids {
        sqlx::query("DELETE FROM rag_chunks_fts WHERE document_id = ?")
            .bind(document_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    let result = sqlx::query("DELETE FROM rag_documents WHERE scope LIKE ?")
        .bind(format!("{}%", CONVERSATION_SCOPE_PREFIX))
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(result.rows_affected() as usize)
}

async fn remove_scope_documents(app: &AppHandle, scope: &str) -> Result<usize, String> {
    let pool = db::get_pool(app).await?;
    let document_ids = sqlx::query("SELECT id FROM rag_documents WHERE scope = ?")
        .bind(scope)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|row| row.get::<String, _>("id"))
        .collect::<Vec<_>>();

    if document_ids.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for document_id in &document_ids {
        sqlx::query("DELETE FROM rag_chunks_fts WHERE document_id = ?")
            .bind(document_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    let result = sqlx::query("DELETE FROM rag_documents WHERE scope = ?")
        .bind(scope)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(result.rows_affected() as usize)
}
