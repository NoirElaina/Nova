use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false)
}

pub fn tool() -> Tool {
    Tool {
        name: "config_tool".into(),
        description: "Read or update Nova runtime config (model/provider/base_url/api_key) in settings.json.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set", "list_keys", "remove"],
                    "description": "Operation to perform"
                },
                "key": {
                    "type": "string",
                    "description": "Config key, e.g. model, provider, baseUrl, apiKey"
                },
                "value": {
                    "description": "Value used by set action"
                }
            },
            "required": ["action"]
        }),
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("settings.json"))
        .map_err(|e| format!("Failed to resolve app_data_dir for settings: {}", e))
}

fn read_settings_json(path: &PathBuf) -> Result<Value, String> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        let init = json!({
            "apiKey": "",
            "baseUrl": "https://api.anthropic.com/v1",
            "model": "claude-3-5-sonnet-20241022",
            "provider": "anthropic"
        });
        fs::write(path, serde_json::to_string_pretty(&init).unwrap_or_else(|_| "{}".into()))
            .map_err(|e| format!("Failed to init settings file: {}", e))?;
        return Ok(init);
    }

    let content = fs::read_to_string(path).map_err(|e| format!("Failed to read settings file: {}", e))?;
    serde_json::from_str::<Value>(&content).map_err(|e| format!("Invalid JSON in settings file: {}", e))
}

fn write_settings_json(path: &PathBuf, value: &Value) -> Result<(), String> {
    let pretty = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, pretty).map_err(|e| format!("Failed to write settings file: {}", e))
}

pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "config_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let action = match input.get("action").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return "Error: Missing 'action' argument".into(),
    };

    let path = match settings_path(app) {
        Ok(p) => p,
        Err(e) => return format!("Error: {}", e),
    };

    let mut settings = match read_settings_json(&path) {
        Ok(v) => v,
        Err(e) => return format!("Error: {}", e),
    };

    match action {
        "get" => {
            if let Some(key) = input.get("key").and_then(|v| v.as_str()) {
                match settings.get(key) {
                    Some(v) => json!({"ok": true, "key": key, "value": v}).to_string(),
                    None => format!("Error: key '{}' not found", key),
                }
            } else {
                json!({"ok": true, "config": settings}).to_string()
            }
        }
        "set" => {
            let key = match input.get("key").and_then(|v| v.as_str()) {
                Some(k) if !k.trim().is_empty() => k,
                _ => return "Error: Missing 'key' for set action".into(),
            };

            let value = match input.get("value") {
                Some(v) => v.clone(),
                None => return "Error: Missing 'value' for set action".into(),
            };

            if !settings.is_object() {
                settings = json!({});
            }

            if let Some(obj) = settings.as_object_mut() {
                obj.insert(key.to_string(), value.clone());
            }

            match write_settings_json(&path, &settings) {
                Ok(_) => json!({"ok": true, "action": "set", "key": key, "value": value}).to_string(),
                Err(e) => format!("Error: {}", e),
            }
        }
        "list_keys" => {
            if let Some(obj) = settings.as_object() {
                let keys: Vec<String> = obj.keys().cloned().collect();
                json!({"ok": true, "keys": keys}).to_string()
            } else {
                "Error: settings root is not an object".into()
            }
        }
        "remove" => {
            let key = match input.get("key").and_then(|v| v.as_str()) {
                Some(k) if !k.trim().is_empty() => k,
                _ => return "Error: Missing 'key' for remove action".into(),
            };

            if let Some(obj) = settings.as_object_mut() {
                let existed = obj.remove(key).is_some();
                if !existed {
                    return format!("Error: key '{}' not found", key);
                }
            } else {
                return "Error: settings root is not an object".into();
            }

            match write_settings_json(&path, &settings) {
                Ok(_) => json!({"ok": true, "action": "remove", "key": key}).to_string(),
                Err(e) => format!("Error: {}", e),
            }
        }
        _ => "Error: action must be one of get | set | list_keys | remove".into(),
    }
}
