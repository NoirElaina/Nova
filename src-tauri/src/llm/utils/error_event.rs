use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tracing::{error, warn};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackendErrorEvent {
    // 错误来源模块标识。
    pub source: String,
    // 未经改写的错误原文，前端直接展示。
    pub message: String,
    // 可选阶段信息（如 provider.send_request）。
    pub stage: Option<String>,
}

pub fn emit_backend_error(
    app: &AppHandle,
    source: &str,
    message: impl Into<String>,
    stage: Option<&str>,
) {
    let raw_message = message.into();
    // 组装统一错误事件 payload：不再做友好化改写，前端直接展示原文。
    let payload = BackendErrorEvent {
        source: source.to_string(),
        message: raw_message.clone(),
        // stage 从 Option<&str> 映射为 Option<String>。
        stage: stage.map(|s| s.to_string()),
    };

    error!(
        source = %payload.source,
        stage = %payload.stage.as_deref().unwrap_or("-"),
        message = %raw_message,
        "backend error"
    );
    // 广播后端错误事件给前端；失败不阻断主流程。
    let _ = app.emit("backend-error", payload.clone());
    // 同步写 stderr 便于本地调试和日志采集。
    eprintln!(
        "[backend-error] source={} stage={} message={}",
        payload.source,
        // stage 为空时打印占位符 "-"。
        payload.stage.as_deref().unwrap_or("-"),
        raw_message
    );
}

pub fn report_backend_result<T>(
    app: &AppHandle,
    source: &str,
    result: Result<T, String>,
    stage: Option<&str>,
) -> Result<T, String> {
    if let Err(error) = &result {
        emit_backend_error(app, source, error.clone(), stage);
    }
    result
}

/// 后端警告：不阻断流程，但必须让用户可见（前端 backend-warning 监听弹提示）。
/// 用于"容忍但丢了信息"的场景，如流里收到未识别的事件类型被忽略。
pub fn emit_backend_warning(
    app: &AppHandle,
    source: &str,
    message: impl Into<String>,
    stage: Option<&str>,
) {
    let raw_message = message.into();
    let payload = BackendErrorEvent {
        source: source.to_string(),
        message: raw_message.clone(),
        stage: stage.map(|s| s.to_string()),
    };

    warn!(
        source = %payload.source,
        stage = %payload.stage.as_deref().unwrap_or("-"),
        message = %raw_message,
        "backend warning"
    );
    // 广播后端警告事件给前端；失败不阻断主流程。
    let _ = app.emit("backend-warning", payload.clone());
    eprintln!(
        "[backend-warning] source={} stage={} message={}",
        payload.source,
        payload.stage.as_deref().unwrap_or("-"),
        raw_message
    );
}
