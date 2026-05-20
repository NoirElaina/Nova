use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tracing::error;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackendErrorEvent {
    // 错误来源模块标识。
    pub source: String,
    // 统一错误码。
    pub code: String,
    // 展示标题。
    pub title: String,
    // 提供给前端展示的处理后消息正文。
    pub message: String,
    // 可选阶段信息（如 provider.send_request）。
    pub stage: Option<String>,
}

fn classify_backend_error(
    source: &str,
    stage: Option<&str>,
    raw_message: &str,
) -> (String, String, String) {
    let source_key = source.trim().to_ascii_lowercase();
    let stage_key = stage.unwrap_or_default().trim().to_ascii_lowercase();
    let raw_key = raw_message.trim().to_ascii_lowercase();

    if raw_key.contains("support image input") || raw_key.contains("image input") {
        return (
            "model_image_unsupported".to_string(),
            "当前模型不支持图片输入".to_string(),
            "请切换到支持图片输入的模型，或移除图片后再发送。".to_string(),
        );
    }

    if raw_key.contains("401") || raw_key.contains("unauthorized") || raw_key.contains("api key") {
        return (
            "provider_auth_failed".to_string(),
            "模型服务认证失败".to_string(),
            "请检查 API Key、Provider 配置或账号权限后再试。".to_string(),
        );
    }

    if raw_key.contains("403") || raw_key.contains("forbidden") {
        return (
            "provider_forbidden".to_string(),
            "模型服务拒绝了本次请求".to_string(),
            "当前账号或模型权限不足，请检查服务端授权配置。".to_string(),
        );
    }

    if raw_key.contains("429")
        || raw_key.contains("rate limit")
        || raw_key.contains("too many requests")
    {
        return (
            "provider_rate_limited".to_string(),
            "模型服务当前较忙".to_string(),
            "请求频率过高或服务限流，请稍后再试。".to_string(),
        );
    }

    if raw_key.contains("timed out")
        || raw_key.contains("timeout")
        || raw_key.contains("deadline exceeded")
    {
        return (
            "provider_timeout".to_string(),
            "请求模型服务超时".to_string(),
            "服务响应时间过长，请稍后重试。".to_string(),
        );
    }

    if raw_key.contains("dns")
        || raw_key.contains("connection refused")
        || raw_key.contains("connection reset")
        || raw_key.contains("network")
        || raw_key.contains("failed to send request")
    {
        return (
            "network_failed".to_string(),
            "无法连接到模型服务".to_string(),
            "请检查网络连接、服务地址或代理配置后再试。".to_string(),
        );
    }

    if raw_key.contains("no such file")
        || raw_key.contains("not found")
        || raw_key.contains("系统找不到")
        || raw_key.contains("文件不存在")
    {
        return (
            "resource_not_found".to_string(),
            "需要的资源不存在".to_string(),
            "请确认文件、会话资源或服务端点仍然可用。".to_string(),
        );
    }

    if raw_key.contains("permission denied") || raw_key.contains("access is denied") {
        return (
            "permission_denied".to_string(),
            "当前操作缺少权限".to_string(),
            "请检查文件权限、目录权限或当前运行环境的授权设置。".to_string(),
        );
    }

    if source_key.starts_with("compact.") {
        return (
            "context_compact_failed".to_string(),
            "上下文压缩失败".to_string(),
            "本轮已跳过压缩步骤，并继续尝试完成当前请求。".to_string(),
        );
    }

    if source_key.contains("mcp") {
        return (
            "mcp_failed".to_string(),
            "MCP 服务执行失败".to_string(),
            "请检查 MCP 服务配置、连接状态或工具参数后再试。".to_string(),
        );
    }

    if source_key.contains("settings") {
        return (
            "settings_failed".to_string(),
            "设置操作失败".to_string(),
            "设置没有成功保存或读取，请检查配置内容后重试。".to_string(),
        );
    }

    if source_key.contains("rag") {
        return (
            "rag_failed".to_string(),
            "知识库操作失败".to_string(),
            "请检查导入内容、文件大小或知识库配置后再试。".to_string(),
        );
    }

    if source_key.contains("provider")
        || stage_key.contains("provider")
        || raw_key.contains("api error")
    {
        return (
            "provider_failed".to_string(),
            "模型服务请求失败".to_string(),
            "本次请求没有成功完成，请检查模型配置后稍后重试。".to_string(),
        );
    }

    (
        "backend_failed".to_string(),
        "后端处理失败".to_string(),
        "当前请求未能完成，请稍后重试。".to_string(),
    )
}

pub fn emit_backend_error(
    app: &AppHandle,
    source: &str,
    message: impl Into<String>,
    stage: Option<&str>,
) {
    let raw_message = message.into();
    let (code, title, user_message) = classify_backend_error(source, stage, &raw_message);
    // 组装统一错误事件 payload。
    let payload = BackendErrorEvent {
        // source 转为拥有所有权字符串。
        source: source.to_string(),
        code,
        title,
        message: user_message,
        // stage 从 Option<&str> 映射为 Option<String>。
        stage: stage.map(|s| s.to_string()),
    };

    error!(
        source = %payload.source,
        code = %payload.code,
        stage = %payload.stage.as_deref().unwrap_or("-"),
        message = %raw_message,
        "backend error"
    );
    // 广播后端错误事件给前端；失败不阻断主流程。
    let _ = app.emit("backend-error", payload.clone());
    // 同步写 stderr 便于本地调试和日志采集。
    eprintln!(
        "[backend-error] source={} code={} stage={} message={}",
        payload.source,
        payload.code,
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
