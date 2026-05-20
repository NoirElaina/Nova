// ─────────────────────────────────────────────────────────────────────────────
// turn_log — 将每轮对话的 wire 级请求/响应记录为 JSONL 文件，用于调试。
//
// 日志路径：<app_data_dir>/turn_logs/<conversation_id>.jsonl
// 搜索 @@日志记录 可定位所有调用点。
//
// ─── 三种 Provider 发送的原生格式 ────────────────────────────────────────────
//
// 1. OpenAI  POST /v1/chat/completions
//    - system prompt 作为 messages[0] { role:"system", content:"..." }
//    - 工具调用：assistant 消息携带 tool_calls[]，arguments 为 JSON 字符串
//    - 工具结果：独立 { role:"tool", tool_call_id:"...", content:"..." } 消息
//    - ID 字段：tool_call_id
//    - 示例结构：
//      { model, messages:[{role,content},{role,tool_calls},{role:tool,tool_call_id},...],
//        tools:[...], stream:true, stream_options:{include_usage:true} }
//
// 2. Anthropic  POST /v1/messages
//    - system prompt 作为顶层 system 字段（不在 messages 里）
//    - 内部 Message 结构与 Anthropic 格式几乎一一对应，几乎无转换
//    - 工具调用：assistant content 里的 { type:"tool_use", id, name, input:{} } 块
//    - 工具结果：user content 里的 { type:"tool_result", tool_use_id, content:[{type:"text"}] } 块
//    - ID 字段：tool_use_id
//    - 示例结构：
//      { model, max_tokens, system:"...",
//        messages:[{role:"user",content:[{type:"text"}]},
//                  {role:"assistant",content:[{type:"tool_use",id,name,input}]},
//                  {role:"user",content:[{type:"tool_result",tool_use_id,content}]}],
//        tools:[...], stream:true }
//
// 3. Responses  POST /v1/responses
//    - system prompt 作为顶层 instructions 字段
//    - 消息容器字段名为 input（不是 messages）
//    - 消息、工具调用、工具结果全部平铺为顶层 item，不嵌套
//    - 工具调用：{ type:"function_call", call_id, name, arguments:"..." }
//    - 工具结果：{ type:"function_call_output", call_id, output:"..." }
//    - 用户消息：{ type:"message", role:"user", content:[{type:"input_text",text}] }
//    - ID 字段：call_id
//    - 示例结构：
//      { model, instructions:"...",
//        input:[{type:"message",role:"user",...},
//               {type:"function_call",call_id,name,arguments},
//               {type:"function_call_output",call_id,output},
//               {type:"message",role:"assistant",...}],
//        tools:[...], stream:true }
// ─────────────────────────────────────────────────────────────────────────────

use std::fs::{self, OpenOptions};
use std::io::Write as IoWrite;

use tauri::{AppHandle, Manager};

/// 设为 false 可全局关闭 turn_log 日志输出。
const TURN_LOG_ENABLED: bool = true;

fn log_path(app: &AppHandle, conversation_id: Option<&str>) -> Option<std::path::PathBuf> {
    let base = app.path().app_data_dir().ok()?;
    let dir = base.join("turn_logs");
    fs::create_dir_all(&dir).ok()?;
    let filename = conversation_id
        .map(|id| {
            let safe: String = id
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("{}.jsonl", safe)
        })
        .unwrap_or_else(|| "default.jsonl".to_string());
    Some(dir.join(filename))
}

fn append_to_log(app: &AppHandle, conversation_id: Option<&str>, text: &str) {
    if !TURN_LOG_ENABLED {
        return;
    }
    let Some(path) = log_path(app, conversation_id) else {
        return;
    };
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[turn_log] 无法打开日志文件 {:?}: {}", path, e);
            return;
        }
    };
    if let Err(e) = writeln!(file, "{}", text) {
        eprintln!("[turn_log] 写入日志失败: {}", e);
    }
}

pub fn log_wire_request(app: &AppHandle, conversation_id: Option<&str>, url: &str, body: &str) {
    let entry = serde_json::json!({
        "type": "wire_request",
        "ts": chrono::Local::now().to_rfc3339(),
        "url": url,
        "body": serde_json::from_str::<serde_json::Value>(body).unwrap_or(serde_json::Value::String(body.to_string())),
    });
    append_to_log(app, conversation_id, &entry.to_string());
}

pub fn log_wire_response(
    app: &AppHandle,
    conversation_id: Option<&str>,
    data: &str,
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
) {
    let entry = serde_json::json!({
        "type": "wire_response",
        "ts": chrono::Local::now().to_rfc3339(),
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "data": serde_json::from_str::<serde_json::Value>(data).unwrap_or(serde_json::Value::String(data.to_string())),
    });
    append_to_log(app, conversation_id, &entry.to_string());
}
