use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::llm::commands::memory;
use crate::llm::commands::types::{
    ConversationMeta, HistoryMessage, HistoryToolExecution,
};
use crate::llm::types::{Content, Message, Role};

// Build sqlite database URL under app data directory.
// Format: sqlite:<path>?mode=rwc (read/write/create).
fn get_db_url(app: &AppHandle) -> Result<String, String> {
    let db_path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("history.db");

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    Ok(format!("sqlite:{}?mode=rwc", db_path.display()))
}

static DB_POOL: OnceCell<SqlitePool> = OnceCell::const_new();

/// 获取历史库连接池（含 schema 保证）。session_log 等模块复用同一连接池。
pub async fn history_pool(app: &AppHandle) -> Result<SqlitePool, String> {
    get_pool_with_schema(app).await
}

// Create a sqlx sqlite pool for history DB.
async fn get_pool(app: &AppHandle) -> Result<SqlitePool, String> {
    let pool = DB_POOL
        .get_or_try_init(|| async {
            let db_url = get_db_url(app)?;
            let pool = SqlitePool::connect(&db_url)
                .await
                .map_err(|e| e.to_string())?;
            ensure_schema(&pool).await?;
            Ok::<SqlitePool, String>(pool)
        })
        .await?;
    Ok(pool.clone())
}

// Ensure required schema exists.
// 事件日志时代仅保留三张表：
// - conversations：会话元数据；
// - session_events：会话事实唯一事实源（append-only，结构见 session_log/store.rs）；
// - token_usage_log：计费流水。
async fn ensure_schema(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            pinned_at INTEGER,
            workspace_path TEXT,
            active_agent_id TEXT
        );
        "#,
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS token_usage_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT,
            model TEXT NOT NULL,
            provider TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd TEXT,
            source TEXT,
            created_at INTEGER NOT NULL
        );
    
        CREATE INDEX IF NOT EXISTS idx_token_usage_log_created
            ON token_usage_log(created_at DESC);
    
        CREATE INDEX IF NOT EXISTS idx_token_usage_log_model
            ON token_usage_log(model);
    
        CREATE INDEX IF NOT EXISTS idx_token_usage_log_conversation
            ON token_usage_log(conversation_id);
        "#,
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 会话事件日志表（唯一事实源；结构定义见 session_log/store.rs）。
    sqlx::query(crate::llm::session_log::store::SESSION_EVENTS_SCHEMA)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    // 会话级智能体：旧库补列（已存在则忽略报错）。
    sqlx::query("ALTER TABLE conversations ADD COLUMN active_agent_id TEXT")
        .execute(pool)
        .await
        .ok();

    Ok(())
}

// Public helper used by command handlers:
// open DB pool and guarantee schema is ready before query.
pub async fn get_pool_with_schema(app: &AppHandle) -> Result<SqlitePool, String> {
    get_pool(app).await
}

pub async fn list_memory_entries(app: &AppHandle) -> Result<Vec<String>, String> {
    crate::llm::services::memory_dir::memory_list(app).await
}

pub async fn add_memory_entry(app: &AppHandle, content: &str) -> Result<(), String> {
    crate::llm::services::memory_dir::memory_add(app, content).await
}

pub async fn remove_memory_entry(app: &AppHandle, old_text: &str) -> Result<(), String> {
    crate::llm::services::memory_dir::memory_remove(app, old_text).await
}

pub async fn clear_memory_entries(app: &AppHandle) -> Result<(), String> {
    crate::llm::services::memory_dir::memory_clear(app).await
}

fn resolved_conversation_title(current_title: &str, first_user_message: Option<&str>) -> String {
    let trimmed = current_title.trim();
    if !trimmed.is_empty() && trimmed != "New chat" {
        return trimmed.to_string();
    }

    first_user_message
        .map(memory::derive_title_from_message)
        .unwrap_or_else(|| trimmed.to_string())
}

fn sanitize_export_file_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            ch if ch.is_control() => '-',
            ch => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch| ch == ' ' || ch == '.')
        .chars()
        .take(80)
        .collect::<String>();

    if sanitized.is_empty() {
        "conversation".to_string()
    } else {
        sanitized
    }
}

fn ensure_json_export_format(format: &str) -> Result<(), String> {
    match format.trim().to_ascii_lowercase().as_str() {
        "json" => Ok(()),
        other => Err(format!("unsupported export format: {}", other)),
    }
}

fn export_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn build_export_path(
    app: &AppHandle,
    title: &str,
    exported_at: chrono::DateTime<chrono::Utc>,
    extension: &str,
) -> Result<PathBuf, String> {
    let mut path = export_dir(app)?;
    path.push(format!(
        "nova-{}-{}.{}",
        sanitize_export_file_name(title),
        exported_at.format("%Y%m%d-%H%M%S"),
        extension
    ));
    Ok(path)
}

struct ConversationExportData {
    title: String,
    messages: Vec<HistoryMessage>,
}

async fn load_conversation_export_data(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<ConversationExportData, String> {
    let pool = get_pool_with_schema(app).await?;

    let title_row: Option<String> =
        sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten();
    let title_row = title_row.ok_or_else(|| "conversation not found".to_string())?;

    let messages = load_history(app, conversation_id).await?;
    // 标题兜底：占位标题时从事件投影出的首条用户消息派生。
    let first_user = messages
        .iter()
        .find(|m| m.role.eq_ignore_ascii_case("user"))
        .map(|m| m.content.clone());
    let title = resolved_conversation_title(&title_row, first_user.as_deref());

    Ok(ConversationExportData { title, messages })
}

async fn ensure_conversation_exists(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let exists = sqlx::query_scalar::<_, i64>("SELECT 1 FROM conversations WHERE id = ? LIMIT 1")
        .bind(conversation_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .is_some();

    if exists {
        Ok(())
    } else {
        Err("conversation not found".to_string())
    }
}

fn chromium_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect()
    }

    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect()
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        vec![
            PathBuf::from("google-chrome"),
            PathBuf::from("microsoft-edge"),
            PathBuf::from("chromium-browser"),
            PathBuf::from("chromium"),
        ]
    }
}

async fn print_html_to_pdf(html_path: &Path, output_path: &Path) -> Result<(), String> {
    let candidates = chromium_candidates();
    if candidates.is_empty() {
        return Err("未找到 Edge/Chrome/Chromium，无法将已渲染 HTML 打印为 PDF。".to_string());
    }

    let mut errors = Vec::new();
    for browser in candidates {
        let output = tokio::process::Command::new(&browser)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-pdf-header-footer")
            .arg(format!("--print-to-pdf={}", output_path.display()))
            .arg(html_path.as_os_str())
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() && output_path.exists() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                errors.push(format!(
                    "{} exited with {}. stderr: {} stdout: {}",
                    browser.display(),
                    output.status,
                    stderr.trim(),
                    stdout.trim()
                ));
            }
            Err(err) => errors.push(format!("{}: {}", browser.display(), err)),
        }
    }

    Err(format!("PDF 导出失败：{}", errors.join("; ")))
}

// Create a new conversation row with generated UUID and optional title.
// workspace_path: 该会话绑定的项目工作区目录；None 时使用内置默认工作区。
pub async fn create_conversation(
    app: &AppHandle,
    title: Option<String>,
    workspace_path: Option<String>,
) -> Result<ConversationMeta, String> {
    let pool = get_pool_with_schema(app).await?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let conv_title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_default();

    // 没传 workspace_path 或传空，就用内置默认工作区（app_data/workspace）下的独立子目录。
    let ws_path = match workspace_path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        Some(p) => p,
        None => {
            let base = crate::command::workspace::default_workspace_root(app)?;
            let conv_dir = base.join(&id);
            std::fs::create_dir_all(&conv_dir)
                .map_err(|e| format!("创建对话工作区失败: {}", e))?;
            let canonical = conv_dir.canonicalize()
                .map_err(|e| format!("无法解析对话工作区: {}", e))?;
            crate::command::workspace::display_path_string(&canonical)
        }
    };

    // 会话元数据单行写入；消息内容后续由会话事件日志承担。
    sqlx::query(
        "INSERT INTO conversations (id, title, created_at, updated_at, workspace_path) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&conv_title)
    .bind(now)
    .bind(now)
    .bind(&ws_path)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;

    // 写入进程内缓存，供同步热路径读取。
    crate::command::workspace::cache_conversation_workspace(&id, &ws_path);

    Ok(ConversationMeta {
        id,
        title: conv_title,
        updated_at: now,
        pinned_at: None,
        workspace_path: Some(ws_path),
        active_agent_id: None,
    })
}

// List conversations ordered by latest update time.
pub async fn list_conversations(app: &AppHandle) -> Result<Vec<ConversationMeta>, String> {
    let pool = get_pool_with_schema(app).await?;

    let rows = sqlx::query(
        r#"
        SELECT
            c.id,
            c.title,
            c.updated_at,
            c.pinned_at,
            c.workspace_path,
            c.active_agent_id
        FROM conversations c
        ORDER BY
            CASE WHEN c.pinned_at IS NULL THEN 1 ELSE 0 END ASC,
            c.pinned_at DESC,
            c.updated_at DESC
        "#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let items: Vec<ConversationMeta> = rows
        .into_iter()
        .map(|row| {
            let ws_path = row
                .get::<Option<String>, _>("workspace_path")
                .filter(|p| !p.trim().is_empty());
            ConversationMeta {
                id: row.get::<String, _>("id"),
                // 标题在首条用户消息发送时已派生落库，这里只做占位规范化。
                title: resolved_conversation_title(&row.get::<String, _>("title"), None),
                updated_at: row.get::<i64, _>("updated_at"),
                pinned_at: row.get::<Option<i64>, _>("pinned_at"),
                workspace_path: ws_path,
                active_agent_id: row
                    .get::<Option<String>, _>("active_agent_id")
                    .filter(|v| !v.trim().is_empty()),
            }
        })
        .collect();

    // 批量刷新进程内缓存，供同步热路径读取。
    let cache_entries: Vec<(String, Option<String>)> = items
        .iter()
        .map(|c| (c.id.clone(), c.workspace_path.clone()))
        .collect();
    crate::command::workspace::refresh_workspace_cache(&cache_entries).await;

    // 会话级智能体缓存同样批量刷新（写穿透模式的兜底刷新点之一）。
    let agent_entries: Vec<(String, Option<String>)> = rows_agent_entries(&items);
    crate::llm::services::agent_bundles::refresh_conversation_agent_cache(&agent_entries);

    Ok(items)
}

// 从行数据提取 (会话id, 智能体id) 对，供缓存批量刷新。
fn rows_agent_entries(items: &[ConversationMeta]) -> Vec<(String, Option<String>)> {
    items
        .iter()
        .map(|c| (c.id.clone(), c.active_agent_id.clone()))
        .collect()
}

/// 设置/移除会话挂载的智能体（bundle_id 为 None = 回到默认 Nova）。
/// 写库后同步更新进程内缓存（写穿透）。
pub async fn set_conversation_agent(
    app: &AppHandle,
    conversation_id: &str,
    bundle_id: Option<&str>,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return Err("conversation_id is required".to_string());
    }

    let agent = bundle_id.map(str::trim).filter(|v| !v.is_empty());
    sqlx::query("UPDATE conversations SET active_agent_id = ? WHERE id = ?")
        .bind(&agent)
        .bind(normalized)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    // 写穿透：立刻同步缓存，provider 同步热路径马上可见。
    crate::llm::services::agent_bundles::cache_conversation_agent(normalized, agent);
    Ok(())
}

/// 读取会话挂载的智能体 id（无会话/未挂载返回 None）。
pub async fn get_conversation_agent(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized = conversation_id.trim();
    if normalized.is_empty() {
        return Ok(None);
    }

    let agent: Option<Option<String>> =
        sqlx::query_scalar("SELECT active_agent_id FROM conversations WHERE id = ?")
            .bind(normalized)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(agent.flatten().filter(|v| !v.trim().is_empty()))
}

/// 删除 bundle 后清空所有引用它的会话（回到默认 Nova），并刷新缓存。
pub async fn clear_conversation_agent_references(
    app: &AppHandle,
    bundle_id: &str,
) -> Result<Vec<String>, String> {
    let pool = get_pool_with_schema(app).await?;
    let normalized = bundle_id.trim();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }

    // 先找出受影响会话，再清引用（不 bump updated_at，避免会话列表顺序被打乱）。
    let affected: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM conversations WHERE active_agent_id = ?",
    )
    .bind(normalized)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query("UPDATE conversations SET active_agent_id = NULL WHERE active_agent_id = ?")
        .bind(normalized)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    for conversation_id in &affected {
        crate::llm::services::agent_bundles::cache_conversation_agent(conversation_id, None);
    }

    Ok(affected)
}

pub async fn set_conversation_pinned(
    app: &AppHandle,
    conversation_id: &str,
    pinned: bool,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let pinned_at = pinned.then(|| chrono::Utc::now().timestamp());

    let result = sqlx::query("UPDATE conversations SET pinned_at = ? WHERE id = ?")
        .bind(pinned_at)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    if result.rows_affected() == 0 {
        return Err("conversation not found".to_string());
    }

    Ok(())
}

pub async fn export_conversation(
    app: &AppHandle,
    conversation_id: &str,
    format: &str,
) -> Result<String, String> {
    ensure_json_export_format(format)?;
    let data = load_conversation_export_data(app, conversation_id).await?;
    let exported_at = chrono::Utc::now();

    let output_path = build_export_path(app, &data.title, exported_at, "json")?;
    let payload = serde_json::json!({
        "id": conversation_id,
        "title": data.title,
        "exportedAt": exported_at.to_rfc3339(),
        "messages": data.messages,
    });
    let body = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    tokio::fs::write(&output_path, body)
        .await
        .map_err(|e| e.to_string())?;

    Ok(output_path.display().to_string())
}

pub async fn export_rendered_conversation_pdf(
    app: &AppHandle,
    conversation_id: &str,
    title: &str,
    html: &str,
) -> Result<String, String> {
    if html.trim().is_empty() {
        return Err("rendered html is empty".to_string());
    }

    ensure_conversation_exists(app, conversation_id).await?;
    let exported_at = chrono::Utc::now();
    let output_path = build_export_path(app, title, exported_at, "pdf")?;

    let mut temp_dir = app
        .path()
        .app_cache_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| e.to_string())?;
    temp_dir.push("exports");
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
    temp_dir.push(format!(
        "{}-{}.html",
        conversation_id,
        exported_at.format("%Y%m%d-%H%M%S")
    ));

    tokio::fs::write(&temp_dir, html)
        .await
        .map_err(|e| e.to_string())?;
    let print_result = print_html_to_pdf(&temp_dir, &output_path).await;
    let _ = tokio::fs::remove_file(&temp_dir).await;
    print_result?;

    Ok(output_path.display().to_string())
}

/// 会话活跃度与标题维护（事件日志时代的持久化入口）：
/// 刷新 updated_at；user_text 非空且标题仍为占位时从首条用户消息派生标题。
pub async fn refresh_conversation_activity(
    app: &AppHandle,
    conversation_id: &str,
    user_text: Option<&str>,
) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    let Some(text) = user_text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(());
    };

    let current_title: Option<String> =
        sqlx::query_scalar("SELECT title FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    let should_update = current_title
        .as_deref()
        .map(|title| {
            let trimmed = title.trim();
            trimmed.is_empty() || trimmed == "New chat"
        })
        .unwrap_or(false);
    if !should_update {
        return Ok(());
    }

    let new_title = memory::derive_title_from_message(text);
    sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(&new_title)
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let title_event = crate::llm::session_log::SessionEvent::TitleChanged { title: new_title };
    if let Err(error) =
        crate::llm::session_log::append_event(app, conversation_id, None, &title_event).await
    {
        tracing::warn!(error = %error, "title_changed event append failed");
    }
    Ok(())
}

/// 事件驱动的纯文本消息追加（分支转存等场景）：写事件日志并维护标题/活跃度。
pub async fn append_plain_chat_message(
    app: &AppHandle,
    conversation_id: &str,
    role: &str,
    content: &str,
) -> Result<(), String> {
    let message_role = if role.eq_ignore_ascii_case("assistant") {
        Role::Assistant
    } else {
        Role::User
    };
    let message = Message {
        role: message_role.clone(),
        content: Content::Text(content.to_string()),
    };
    let event = crate::llm::session_log::SessionEvent::from_model_message(message);
    crate::llm::session_log::append_event(app, conversation_id, None, &event).await?;
    let user_text = matches!(message_role, Role::User).then_some(content);
    refresh_conversation_activity(app, conversation_id, user_text).await
}

/// 事件驱动的历史重置（编辑消息重发用）：清空该会话事件流，
/// 再把传入消息序列重写为事件（用户消息保留附件，助手消息保留 token/成本）。
pub async fn reset_conversation_messages(
    app: &AppHandle,
    conversation_id: &str,
    messages: &[HistoryMessage],
) -> Result<(), String> {
    crate::llm::session_log::delete_events(app, conversation_id).await?;

    let mut events = Vec::with_capacity(messages.len());
    for message in messages {
        let role = if message.role.eq_ignore_ascii_case("assistant") {
            Role::Assistant
        } else {
            Role::User
        };
        let model_message = Message {
            role: role.clone(),
            content: Content::Text(message.content.clone()),
        };
        let event = match role {
            Role::User => crate::llm::session_log::SessionEvent::UserMessage {
                message: model_message,
                attachments: message.attachments.clone(),
            },
            Role::Assistant => crate::llm::session_log::SessionEvent::AssistantMessage {
                message: model_message,
                token_usage: message.token_usage,
                cost: message.cost.clone(),
            },
        };
        events.push(event);
    }
    crate::llm::session_log::append_events(app, conversation_id, None, &events).await?;

    let first_user = messages
        .iter()
        .find(|m| m.role.eq_ignore_ascii_case("user"))
        .map(|m| m.content.as_str());
    refresh_conversation_activity(app, conversation_id, first_user).await
}

// Load all persisted messages for a conversation in stable chronological order.
// 纯事件投影：从会话事件日志渲染（旧数据不兼容，无事件的会话返回空）。
pub async fn load_history(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String> {
    let events = crate::llm::session_log::load_events(app, conversation_id).await?;
    Ok(crate::llm::session_log::projection::render_ui_history(&events))
}

pub async fn load_conversation_tool_logs(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Vec<HistoryToolExecution>, String> {
    // 纯事件投影：从 ToolCall/ToolResult 事件配对渲染。
    let events = crate::llm::session_log::load_events(app, conversation_id).await?;
    Ok(crate::llm::session_log::projection::render_tool_logs(&events))
}

// Clear history data.
// - with conversation_id: clear the scoped conversation's persisted data
// - without conversation_id: clear all persisted history and conversation rows
pub async fn clear_history(app: &AppHandle, conversation_id: Option<String>) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    if let Some(id) = conversation_id {
        sqlx::query("DELETE FROM session_events WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE token_usage_log SET conversation_id = NULL WHERE conversation_id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::command::session_files::delete_all_session_files(app, &id).await?;
        let _ = crate::llm::services::plan_files::delete_conversation_plan(app, Some(&id));
        crate::llm::services::shell_sessions::close_session(Some(&id)).await;
        let _ = crate::llm::services::user_terminal::stop_session(Some(&id));
        // 内存级 per-conversation 缓存统一回收（注册表单一入口）。
        crate::llm::utils::cache_registry::clear_conversation_caches(Some(&id));
    } else {
        sqlx::query("DELETE FROM session_events")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE token_usage_log SET conversation_id = NULL")
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
        crate::command::session_files::delete_all_session_files_all(app).await?;
        crate::llm::services::shell_sessions::close_all_sessions().await;
        crate::llm::services::user_terminal::close_all_sessions();
        // 全量清除路径：所有会话级内存缓存一并清空。
        crate::llm::utils::cache_registry::clear_conversation_caches(None);
    }

    Ok(())
}

// Delete one conversation and all dependent rows.
pub async fn delete_conversation(app: &AppHandle, conversation_id: &str) -> Result<(), String> {
    let pool = get_pool_with_schema(app).await?;

    // Check if the conversation has an auto-generated workspace to clean up
    let ws_path_opt: Option<String> = sqlx::query_scalar("SELECT workspace_path FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .flatten();

    if let Some(ws_path) = ws_path_opt {
        if let Ok(default_root) = crate::command::workspace::default_workspace_root(app) {
            let auto_dir = default_root.join(conversation_id);
            if let Ok(canonical_auto) = auto_dir.canonicalize() {
                if ws_path == crate::command::workspace::display_path_string(&canonical_auto) {
                    let _ = std::fs::remove_dir_all(&auto_dir);
                }
            }
        }
    }

    sqlx::query("DELETE FROM session_events WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("UPDATE token_usage_log SET conversation_id = NULL WHERE conversation_id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(conversation_id)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    crate::command::session_files::delete_all_session_files(app, conversation_id).await?;
    let _ = crate::llm::services::plan_files::delete_conversation_plan(app, Some(conversation_id));
    crate::llm::services::shell_sessions::close_session(Some(conversation_id)).await;
    let _ = crate::llm::services::user_terminal::stop_session(Some(conversation_id));

    // 内存级 per-conversation 缓存统一回收（注册表单一入口）。
    crate::llm::utils::cache_registry::clear_conversation_caches(Some(conversation_id));

    Ok(())
}
