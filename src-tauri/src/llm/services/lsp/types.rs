use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspStatusResponse {
    pub workspace_root: String,
    pub servers: Vec<LspServerStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspServerStatus {
    pub language_id: String,
    pub display_name: String,
    pub command: Option<String>,
    pub available: bool,
    pub running: bool,
    pub diagnostic_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspDiagnosticsResponse {
    pub workspace_root: String,
    pub server: Option<String>,
    pub diagnostics: Vec<LspDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LspDiagnostic {
    pub uri: String,
    pub path: String,
    pub relative_path: String,
    pub message: String,
    pub severity: Option<u64>,
    pub source: Option<String>,
    pub code: Option<String>,
    pub line: u64,
    pub character: u64,
    pub end_line: u64,
    pub end_character: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspRequestResponse {
    pub workspace_root: String,
    pub server: String,
    pub result: Value,
    pub locations: Vec<LspLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspSymbolsResponse {
    pub workspace_root: String,
    pub server: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspHoverResponse {
    pub workspace_root: String,
    pub server: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspLocation {
    pub uri: String,
    pub path: String,
    pub relative_path: String,
    pub line: u64,
    pub character: u64,
    pub end_line: u64,
    pub end_character: u64,
}
