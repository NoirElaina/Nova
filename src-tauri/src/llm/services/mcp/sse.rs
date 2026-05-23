use std::collections::HashMap;

pub(super) fn connect_sse(_url: &str, _headers: &HashMap<String, String>) -> Result<(), String> {
    Err("SSE MCP runtime not implemented yet. Use stdio for now.".to_string())
}
