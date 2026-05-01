use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const RAG_STORE_VERSION: u32 = 1;
const MAX_DOCUMENT_CHARS: usize = 200_000;
const MAX_BATCH_SIZE: usize = 200;

fn default_store_version() -> u32 {
    RAG_STORE_VERSION
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagDocumentInput {
    pub source_name: String,
    pub source_type: String,
    pub mime_type: Option<String>,
    pub content: String,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RagDocument {
    pub id: String,
    pub source_name: String,
    pub source_type: String,
    pub mime_type: Option<String>,
    pub conversation_id: Option<String>,
    pub content: String,
    pub content_chars: usize,
    pub checksum: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RagStore {
    #[serde(default = "default_store_version")]
    pub version: u32,
    #[serde(default)]
    pub documents: Vec<RagDocument>,
}

impl Default for RagStore {
    fn default() -> Self {
        Self {
            version: RAG_STORE_VERSION,
            documents: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagDocumentMeta {
    pub id: String,
    pub source_name: String,
    pub source_type: String,
    pub mime_type: Option<String>,
    pub content_chars: usize,
    pub preview: String,
    pub checksum: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagStats {
    pub document_count: usize,
    pub total_chars: usize,
    pub last_updated_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagRejectedItem {
    pub source_name: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagUpsertResult {
    pub added: u32,
    pub updated: u32,
    pub rejected: Vec<RagRejectedItem>,
    pub total_documents: usize,
    pub total_chars: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchHit {
    pub id: String,
    pub source_name: String,
    pub source_type: String,
    pub mime_type: Option<String>,
    pub score: u32,
    pub snippet: String,
    pub content_chars: usize,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagDocumentContent {
    pub id: String,
    pub source_name: String,
    pub source_type: String,
    pub mime_type: Option<String>,
    pub content: String,
    pub content_chars: usize,
    pub checksum: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn rag_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    // RAG 数据文件只使用 app_data_dir，不做候选回退路径。
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("rag").join("documents.json"))
        .map_err(|e| format!("Failed to resolve app_data_dir for RAG store: {}", e))
}

fn load_store(app: &AppHandle) -> Result<RagStore, String> {
    let path = rag_store_path(app)?;
    if !path.exists() {
        return Ok(RagStore::default());
    }

    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if content.trim().is_empty() {
        return Ok(RagStore::default());
    }

    let mut store = serde_json::from_str::<RagStore>(&content)
        .map_err(|e| format!("Failed to parse RAG store: {}", e))?;
    if store.version == 0 {
        store.version = RAG_STORE_VERSION;
    }
    Ok(store)
}

fn save_store(app: &AppHandle, store: &RagStore) -> Result<(), String> {
    let path = rag_store_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let content = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())
}

fn normalize_content(raw: &str) -> String {
    raw.replace("\r\n", "\n").trim().to_string()
}

fn normalize_source_type(raw: &str) -> String {
    let key = raw.trim().to_ascii_lowercase();
    if key.is_empty() {
        "text".to_string()
    } else {
        key
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_optional_conversation_id(value: Option<String>) -> Option<String> {
    normalize_optional_string(value)
}

fn normalize_source_name(raw: &str, fallback_index: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        format!("document-{}", fallback_index + 1)
    } else {
        trimmed.to_string()
    }
}

fn preview_text(content: &str) -> String {
    let compact = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let mut chars = compact.chars();
    let preview: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{}...", preview)
    } else {
        preview
    }
}

fn fnv1a_64_hex(input: &str) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:016x}", hash)
}

fn calculate_stats(docs: &[RagDocument]) -> RagStats {
    let total_chars = docs.iter().map(|d| d.content_chars).sum::<usize>();
    let last_updated_at = docs.iter().map(|d| d.updated_at).max();
    RagStats {
        document_count: docs.len(),
        total_chars,
        last_updated_at,
    }
}

fn split_query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
}

fn calculate_search_score(source_name: &str, content: &str, terms: &[String]) -> u32 {
    let mut score = 0u32;
    let source_lower = source_name.to_lowercase();
    let content_lower = content.to_lowercase();

    for term in terms {
        if source_lower.contains(term) {
            score += 5;
        }
        score += content_lower.match_indices(term).count() as u32;
    }

    score
}

fn rag_document_matches_scope(doc: &RagDocument, conversation_id: Option<&str>) -> bool {
    match conversation_id {
        Some(scope_id) => doc.conversation_id.as_deref() == Some(scope_id),
        None => doc.conversation_id.is_none(),
    }
}

fn rag_search_documents_with_scope(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
    conversation_id: Option<String>,
) -> Result<Vec<RagSearchHit>, String> {
    let query_text = query.trim();
    if query_text.is_empty() {
        return Err("query is required".to_string());
    }

    let terms = split_query_terms(query_text);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let scope = normalize_optional_conversation_id(conversation_id);
    let max_hits = limit.unwrap_or(5).clamp(1, 50);
    let store = load_store(&app)?;

    let mut hits = store
        .documents
        .into_iter()
        .filter_map(|doc| {
            if !rag_document_matches_scope(&doc, scope.as_deref()) {
                return None;
            }

            let score = calculate_search_score(&doc.source_name, &doc.content, &terms);
            if score == 0 {
                return None;
            }

            Some(RagSearchHit {
                id: doc.id,
                source_name: doc.source_name,
                source_type: doc.source_type,
                mime_type: doc.mime_type,
                score,
                snippet: preview_text(&doc.content),
                content_chars: doc.content_chars,
                updated_at: doc.updated_at,
            })
        })
        .collect::<Vec<_>>();

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    hits.truncate(max_hits);

    Ok(hits)
}

pub fn rag_search_documents(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<RagSearchHit>, String> {
    rag_search_documents_with_scope(app, query, limit, None)
}

pub fn rag_search_conversation_documents(
    app: AppHandle,
    conversation_id: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<RagSearchHit>, String> {
    let normalized_conversation_id = normalize_optional_conversation_id(Some(conversation_id))
        .ok_or_else(|| "conversation_id is required".to_string())?;
    rag_search_documents_with_scope(app, query, limit, Some(normalized_conversation_id))
}

#[tauri::command]
pub fn rag_read_document(
    app: AppHandle,
    document_id: String,
) -> Result<Option<RagDocumentContent>, String> {
    let id = document_id.trim();
    if id.is_empty() {
        return Err("document_id is required".to_string());
    }

    let store = load_store(&app)?;
    let found = store.documents.into_iter().find(|doc| doc.id == id);

    Ok(found.map(|doc| RagDocumentContent {
        id: doc.id,
        source_name: doc.source_name,
        source_type: doc.source_type,
        mime_type: doc.mime_type,
        content: doc.content,
        content_chars: doc.content_chars,
        checksum: doc.checksum,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
    }))
}

#[tauri::command]
pub fn rag_get_stats(app: AppHandle) -> Result<RagStats, String> {
    let store = load_store(&app)?;
    let global_documents = store
        .documents
        .into_iter()
        .filter(|doc| doc.conversation_id.is_none())
        .collect::<Vec<_>>();
    Ok(calculate_stats(&global_documents))
}

#[tauri::command]
pub fn rag_list_documents(app: AppHandle) -> Result<Vec<RagDocumentMeta>, String> {
    let store = load_store(&app)?;
    let mut items = store
        .documents
        .into_iter()
        .filter(|doc| doc.conversation_id.is_none())
        .map(|doc| RagDocumentMeta {
            id: doc.id,
            source_name: doc.source_name,
            source_type: doc.source_type,
            mime_type: doc.mime_type,
            content_chars: doc.content_chars,
            preview: preview_text(&doc.content),
            checksum: doc.checksum,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(items)
}

#[tauri::command]
pub fn rag_list_conversation_documents(
    app: AppHandle,
    conversation_id: String,
) -> Result<Vec<RagDocumentMeta>, String> {
    let scope_id = normalize_optional_conversation_id(Some(conversation_id))
        .ok_or_else(|| "conversation_id is required".to_string())?;

    let store = load_store(&app)?;
    let mut items = store
        .documents
        .into_iter()
        .filter(|doc| doc.conversation_id.as_deref() == Some(scope_id.as_str()))
        .map(|doc| RagDocumentMeta {
            id: doc.id,
            source_name: doc.source_name,
            source_type: doc.source_type,
            mime_type: doc.mime_type,
            content_chars: doc.content_chars,
            preview: preview_text(&doc.content),
            checksum: doc.checksum,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        })
        .collect::<Vec<_>>();

    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(items)
}

#[tauri::command]
pub fn rag_upsert_documents(
    app: AppHandle,
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

    let mut store = load_store(&app)?;
    let now = Utc::now().timestamp();

    let mut added = 0u32;
    let mut updated = 0u32;
    let mut rejected: Vec<RagRejectedItem> = Vec::new();

    for (index, item) in documents.into_iter().enumerate() {
        let source_name = normalize_source_name(&item.source_name, index);
        let content = normalize_content(&item.content);
        let conversation_id = normalize_optional_conversation_id(item.conversation_id);

        if content.is_empty() {
            rejected.push(RagRejectedItem {
                source_name,
                reason: "内容为空".to_string(),
            });
            continue;
        }

        let content_chars = content.chars().count();
        if content_chars > MAX_DOCUMENT_CHARS {
            rejected.push(RagRejectedItem {
                source_name,
                reason: format!("内容过长，最大允许 {} 字符", MAX_DOCUMENT_CHARS),
            });
            continue;
        }

        let checksum = fnv1a_64_hex(&content);
        let source_type = normalize_source_type(&item.source_type);
        let mime_type = normalize_optional_string(item.mime_type);
        let scope_key = conversation_id.clone();

        if let Some(existing) = store
            .documents
            .iter_mut()
            .find(|d| d.checksum == checksum && d.conversation_id == scope_key)
        {
            existing.source_name = source_name;
            existing.source_type = source_type;
            existing.mime_type = mime_type;
            existing.conversation_id = conversation_id;
            existing.content = content;
            existing.content_chars = content_chars;
            existing.updated_at = now;
            updated += 1;
            continue;
        }

        store.documents.push(RagDocument {
            id: Uuid::new_v4().to_string(),
            source_name,
            source_type,
            mime_type,
            conversation_id,
            content,
            content_chars,
            checksum,
            created_at: now,
            updated_at: now,
        });
        added += 1;
    }

    if added > 0 || updated > 0 {
        save_store(&app, &store)?;
    }

    let stats = calculate_stats(&store.documents);
    Ok(RagUpsertResult {
        added,
        updated,
        rejected,
        total_documents: stats.document_count,
        total_chars: stats.total_chars,
    })
}

#[tauri::command]
pub fn rag_upsert_conversation_documents(
    app: AppHandle,
    conversation_id: String,
    documents: Vec<RagDocumentInput>,
) -> Result<RagUpsertResult, String> {
    let normalized_conversation_id = normalize_optional_conversation_id(Some(conversation_id))
        .ok_or_else(|| "conversation_id is required".to_string())?;

    let scoped_documents = documents
        .into_iter()
        .map(|mut doc| {
            doc.conversation_id = Some(normalized_conversation_id.clone());
            doc
        })
        .collect::<Vec<_>>();

    rag_upsert_documents(app, scoped_documents)
}

pub fn rag_remove_conversation_documents(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<usize, String> {
    let normalized_conversation_id = normalize_optional_conversation_id(Some(conversation_id.to_string()));
    let Some(scope_id) = normalized_conversation_id else {
        return Ok(0);
    };

    let mut store = load_store(app)?;
    let before = store.documents.len();
    store.documents
        .retain(|doc| doc.conversation_id.as_deref() != Some(scope_id.as_str()));
    let removed = before.saturating_sub(store.documents.len());

    if removed > 0 {
        save_store(app, &store)?;
    }

    Ok(removed)
}

pub fn rag_remove_all_conversation_documents(app: &AppHandle) -> Result<usize, String> {
    let mut store = load_store(app)?;
    let before = store.documents.len();
    store.documents.retain(|doc| doc.conversation_id.is_none());
    let removed = before.saturating_sub(store.documents.len());

    if removed > 0 {
        save_store(app, &store)?;
    }

    Ok(removed)
}

#[tauri::command]
pub fn rag_remove_document(app: AppHandle, document_id: String) -> Result<bool, String> {
    let id = document_id.trim();
    if id.is_empty() {
        return Err("document_id is required".to_string());
    }

    let mut store = load_store(&app)?;
    let before = store.documents.len();
    store.documents.retain(|doc| doc.id != id);
    let removed = before != store.documents.len();

    if removed {
        save_store(&app, &store)?;
    }

    Ok(removed)
}

#[tauri::command]
pub fn rag_clear_documents(app: AppHandle) -> Result<(), String> {
    let mut store = load_store(&app)?;
    let before = store.documents.len();
    store.documents.retain(|doc| doc.conversation_id.is_some());
    if store.documents.len() == before {
        return Ok(());
    }

    save_store(&app, &store)
}
