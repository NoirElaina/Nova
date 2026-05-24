use crate::llm::tools::{app_tool, AppExecuteFuture, ToolRegistration};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// 把 async 的 `execute_with_app` 包成统一的 `AppExecuteFuture`，方便注册到工具运行时。
// `app` 提供 settings.json 所在目录，`input` 是模型传来的 action/key/value 参数。
fn execute_with_app_boxed(
    app: AppHandle,
    _conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move { execute_with_app(&app, input).await })
}

// 返回 config_tool 的注册信息。
// 这里声明它必须走带 AppHandle 的执行路径，因为读写配置文件需要 app_data_dir。
pub(crate) fn registration() -> ToolRegistration {
    app_tool(tool, execute, execute_with_app_boxed, false, None)
}

// 返回模型可见的 config_tool 元数据。
// schema 里的 `action` 决定本次是读取、写入、列 key 还是删除配置项。
pub fn tool() -> Tool {
    Tool {
        name: "config_tool".into(),
        description: "Read or update Nova runtime config in settings.json.".into(),
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
                    "description": "Config key, e.g. provider, providerProfiles, hookEnv"
                },
                "value": {
                    "description": "Value used by set action"
                }
            },
            "required": ["action"]
        }),
    }
}

// 解析 settings.json 的绝对路径。
// `app` 只用于拿到应用数据目录，最终路径固定拼成 `<app_data_dir>/settings.json`。
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("settings.json"))
        .map_err(|e| format!("Failed to resolve app_data_dir for settings: {}", e))
}

// 读取 settings.json，并在文件不存在时初始化一个默认配置。
// `path` 是 settings.json 的绝对路径；返回值始终是可继续操作的 JSON 对象。
fn read_settings_json(path: &PathBuf) -> Result<Value, String> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        let init = json!({
            "provider": "anthropic",
            "providerProfiles": {
                "anthropic": {
                    "displayName": "Anthropic",
                    "protocol": "anthropic",
                    "apiKey": "",
                    "baseUrl": "https://api.anthropic.com/v1",
                    "model": "claude-3-5-sonnet-20241022"
                }
            },
            "hookEnv": {}
        });
        fs::write(path, serde_json::to_string_pretty(&init).unwrap_or_else(|_| "{}".into()))
            .map_err(|e| format!("Failed to init settings file: {}", e))?;
        return Ok(init);
    }

    let content = fs::read_to_string(path).map_err(|e| format!("Failed to read settings file: {}", e))?;
    serde_json::from_str::<Value>(&content).map_err(|e| format!("Invalid JSON in settings file: {}", e))
}

// 把更新后的 JSON 配置写回 settings.json。
// `value` 是已经修改好的整份配置对象，不在这里做业务判断，只负责落盘。
fn write_settings_json(path: &PathBuf, value: &Value) -> Result<(), String> {
    let pretty = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, pretty).map_err(|e| format!("Failed to write settings file: {}", e))
}

// 这个同步入口只返回错误提示，提醒调用方必须走 `execute_with_app`。
// 因为 config_tool 需要 AppHandle 才能定位配置文件路径。
pub fn execute(_input: Value) -> String {
    json!({
        "ok": false,
        "message": "config_tool requires AppHandle-aware execution and should be routed via execute_tool_with_app."
    })
    .to_string()
}

// 根据 `action` 对 settings.json 执行 get/set/list_keys/remove。
// `action` 决定分支，`path` 是配置文件位置，`settings` 是当前内存中的整份 JSON 配置。
pub async fn execute_with_app(app: &AppHandle, input: Value) -> String {
    let action = match input.get("action").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return json!({ "ok": false, "error": "Missing 'action' argument" }).to_string(),
    };

    // path: 当前用户环境下 settings.json 的绝对路径。
    let path = match settings_path(app) {
        Ok(p) => p,
        Err(e) => return json!({ "ok": false, "error": e }).to_string(),
    };

    // settings: 读出来的整份配置，会在 set/remove 分支里原地修改后再写回磁盘。
    let mut settings = match read_settings_json(&path) {
        Ok(v) => v,
        Err(e) => return json!({ "ok": false, "error": e }).to_string(),
    };

    match action {
        "get" => {
            if let Some(key) = input.get("key").and_then(|v| v.as_str()) {
                match settings.get(key) {
                    Some(v) => json!({"ok": true, "key": key, "value": v}).to_string(),
                    None => json!({ "ok": false, "error": format!("key '{}' not found", key) }).to_string(),
                }
            } else {
                json!({"ok": true, "config": settings}).to_string()
            }
        }
        "set" => {
            let key = match input.get("key").and_then(|v| v.as_str()) {
                Some(k) if !k.trim().is_empty() => k,
                _ => return json!({ "ok": false, "error": "Missing 'key' for set action" }).to_string(),
            };

            let value = match input.get("value") {
                Some(v) => v.clone(),
                None => return json!({ "ok": false, "error": "Missing 'value' for set action" }).to_string(),
            };

            if !settings.is_object() {
                settings = json!({});
            }

            if let Some(obj) = settings.as_object_mut() {
                obj.insert(key.to_string(), value.clone());
            }

            match write_settings_json(&path, &settings) {
                Ok(_) => json!({"ok": true, "action": "set", "key": key, "value": value}).to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        "list_keys" => {
            if let Some(obj) = settings.as_object() {
                let keys: Vec<String> = obj.keys().cloned().collect();
                json!({"ok": true, "keys": keys}).to_string()
            } else {
                json!({ "ok": false, "error": "settings root is not an object" }).to_string()
            }
        }
        "remove" => {
            let key = match input.get("key").and_then(|v| v.as_str()) {
                Some(k) if !k.trim().is_empty() => k,
                _ => return json!({ "ok": false, "error": "Missing 'key' for remove action" }).to_string(),
            };

            if let Some(obj) = settings.as_object_mut() {
                let existed = obj.remove(key).is_some();
                if !existed {
                    return json!({ "ok": false, "error": format!("key '{}' not found", key) }).to_string();
                }
            } else {
                return json!({ "ok": false, "error": "settings root is not an object" }).to_string();
            }

            match write_settings_json(&path, &settings) {
                Ok(_) => json!({"ok": true, "action": "remove", "key": key}).to_string(),
                Err(e) => json!({ "ok": false, "error": e }).to_string(),
            }
        }
        _ => json!({ "ok": false, "error": "action must be one of get | set | list_keys | remove" }).to_string(),
    }
}
