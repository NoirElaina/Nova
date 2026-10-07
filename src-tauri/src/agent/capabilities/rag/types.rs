use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RagDocumentInput {
    pub source_name: String,
    pub source_type: String,
    pub mime_type: Option<String>,
    pub content: String,
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
