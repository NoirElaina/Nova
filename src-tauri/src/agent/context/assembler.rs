use std::collections::HashSet;
use std::path::Path;
use tauri::AppHandle;

use crate::provider::types::{Content, ContentBlock, Message, Role};

pub const CONTEXT_TAG_START: &str = "<workspace_context>";
pub const CONTEXT_TAG_END: &str = "</workspace_context>";

/// 确定性上下文装配器（Deterministic Context Assembler）
///
/// 核心职责：
/// 1. 语法合规性治理：清洗未完成或断裂的孤儿工具块，杜绝上游 400 校验异常；
/// 2. 状态时效性治理：剥除多轮历史中旧的陈旧环境快照，防止上下文冗余膨胀；
/// 3. 工作区全景感知：为当前最新轮次动态注水 Git 状态、技术栈特征与轻量目录骨架；
/// 4. 协议角色交替保障：规整相邻连续 User 消息，保证协议鲁棒性。
pub struct ContextAssembler;

impl ContextAssembler {
    /// 组装发送给 LLM 的标准上下文
    pub async fn assemble(
        app: &AppHandle,
        conversation_id: Option<&str>,
        mut history: Vec<Message>,
    ) -> Result<Vec<Message>, String> {
        if history.is_empty() {
            return Ok(history);
        }

        // 1. 语法合规性清洗：丢弃失去配对的孤儿 ToolUse / ToolResult 块
        crate::agent::capabilities::compact::drop_orphan_tool_blocks(&mut history);

        // 2. 剔除历史消息中先前注入过的旧工作区上下文，避免多轮累积膨胀
        strip_stale_workspace_contexts(&mut history);

        // 3. 构建最新的工作区环境快照并注入最新轮次
        if let Ok(root) = crate::services::workspace::workspace_root_for_conversation(app, conversation_id) {
            let snapshot = build_workspace_snapshot(&root);
            inject_fresh_workspace_context(&mut history, &snapshot);
        }

        // 4. 相邻相同角色合并保护，确保 User/Assistant 交替合规
        let normalized = normalize_role_alternation(history);

        Ok(normalized)
    }
}

/// 剥除历史文本中先前包含的工作区快照
fn strip_context_text(text: &str) -> String {
    if let (Some(start), Some(end)) = (text.find(CONTEXT_TAG_START), text.find(CONTEXT_TAG_END)) {
        if start <= end {
            let end_idx = end + CONTEXT_TAG_END.len();
            let mut cleaned = String::with_capacity(text.len());
            cleaned.push_str(&text[..start]);
            cleaned.push_str(&text[end_idx..]);
            return cleaned.trim().to_string();
        }
    }
    text.to_string()
}

/// 清理历史 User 消息中的陈旧上下文块
fn strip_stale_workspace_contexts(history: &mut [Message]) {
    for msg in history.iter_mut() {
        if msg.role != Role::User {
            continue;
        }
        match &mut msg.content {
            Content::Text(text) => {
                let cleaned = strip_context_text(text);
                if cleaned != *text {
                    *text = cleaned;
                }
            }
            Content::Blocks(blocks) => {
                for block in blocks.iter_mut() {
                    if let ContentBlock::Text { text } = block {
                        let cleaned = strip_context_text(text);
                        if cleaned != *text {
                            *text = cleaned;
                        }
                    }
                }
            }
        }
    }
}

/// 扫描工作区根目录，生成高信息密度、极轻量的工作区环境快照
pub fn build_workspace_snapshot(root: &Path) -> String {
    let mut lines = Vec::new();
    lines.push(CONTEXT_TAG_START.to_string());
    lines.push(format!("Root: {}", root.display()));

    // 1. Git 状态感知
    if let Some(git) = crate::services::git_ops::compact_git_summary(root) {
        lines.push(format!("Git: {}", git));
    }

    // 2. 关键技术栈特征检测
    let mut detected_stacks = Vec::new();
    if root.join("Cargo.toml").exists() {
        detected_stacks.push("Rust (Cargo.toml)");
    }
    if root.join("package.json").exists() {
        detected_stacks.push("Node.js/Web (package.json)");
    }
    if root.join("go.mod").exists() {
        detected_stacks.push("Go (go.mod)");
    }
    if root.join("pyproject.toml").exists() || root.join("requirements.txt").exists() {
        detected_stacks.push("Python");
    }
    if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
        detected_stacks.push("Java/JVM");
    }
    if root.join("CMakeLists.txt").exists() {
        detected_stacks.push("C/C++ (CMake)");
    }

    if !detected_stacks.is_empty() {
        lines.push(format!("Tech Stack: {}", detected_stacks.join(", ")));
    }

    // 3. 顶层轻量目录与文件骨架
    let ignored: HashSet<&str> = [
        ".git", "node_modules", "target", "dist", "build", ".next", ".nuxt",
        ".turbo", ".cache", ".vscode", ".idea", "venv", ".venv", "__pycache__",
        "coverage", ".gradle"
    ].into_iter().collect();

    if let Ok(entries) = std::fs::read_dir(root) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if ignored.contains(name.as_str()) || name.starts_with('.') {
                continue;
            }
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    dirs.push(format!("{name}/"));
                } else if ft.is_file() {
                    files.push(name);
                }
            }
        }
        dirs.sort();
        files.sort();

        let mut layout_parts = Vec::new();
        if !dirs.is_empty() {
            let shown_dirs: Vec<String> = dirs.iter().take(8).cloned().collect();
            let more = dirs.len().saturating_sub(8);
            let dir_str = if more > 0 {
                format!("{}+{} dirs", shown_dirs.join(" "), more)
            } else {
                shown_dirs.join(" ")
            };
            layout_parts.push(format!("[dirs] {}", dir_str));
        }
        if !files.is_empty() {
            let shown_files: Vec<String> = files.iter().take(10).cloned().collect();
            let more = files.len().saturating_sub(10);
            let file_str = if more > 0 {
                format!("{}+{} files", shown_files.join(", "), more)
            } else {
                shown_files.join(", ")
            };
            layout_parts.push(format!("[files] {}", file_str));
        }

        if !layout_parts.is_empty() {
            lines.push(format!("Top-level: {}", layout_parts.join(" | ")));
        }
    }

    lines.push(CONTEXT_TAG_END.to_string());
    lines.join("\n")
}

/// 将最新快照注入最后一条 User 消息前缀
fn inject_fresh_workspace_context(history: &mut [Message], snapshot: &str) {
    if let Some(last_user_idx) = history.iter().rposition(|m| m.role == Role::User) {
        let msg = &mut history[last_user_idx];
        match &mut msg.content {
            Content::Text(text) => {
                if text.trim().is_empty() {
                    *text = snapshot.to_string();
                } else {
                    *text = format!("{}\n\n{}", snapshot, text);
                }
            }
            Content::Blocks(blocks) => {
                if let Some(ContentBlock::Text { text }) = blocks.iter_mut().find(|b| matches!(b, ContentBlock::Text { .. })) {
                    if text.trim().is_empty() {
                        *text = snapshot.to_string();
                    } else {
                        *text = format!("{}\n\n{}", snapshot, text);
                    }
                } else {
                    blocks.insert(0, ContentBlock::Text { text: snapshot.to_string() });
                }
            }
        }
    }
}

/// 保护模型协议交替合规性：平滑合并相邻的连续 User 消息
fn normalize_role_alternation(history: Vec<Message>) -> Vec<Message> {
    let mut normalized: Vec<Message> = Vec::with_capacity(history.len());

    for msg in history {
        if let Some(last) = normalized.last_mut() {
            if last.role == msg.role && msg.role == Role::User {
                let mut combined_blocks = Vec::new();
                match &last.content {
                    Content::Text(t) => combined_blocks.push(ContentBlock::Text { text: t.clone() }),
                    Content::Blocks(b) => combined_blocks.extend(b.clone()),
                }
                match msg.content {
                    Content::Text(t) => combined_blocks.push(ContentBlock::Text { text: t }),
                    Content::Blocks(b) => combined_blocks.extend(b),
                }
                last.content = Content::Blocks(combined_blocks);
                continue;
            }
        }
        normalized.push(msg);
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_stale_workspace_contexts() {
        let mut messages = vec![
            Message {
                role: Role::User,
                content: Content::Text(format!(
                    "{}\nRoot: /old\nGit: branch old{}\n\n用户原本的问题",
                    CONTEXT_TAG_START, CONTEXT_TAG_END
                )),
            },
            Message {
                role: Role::Assistant,
                content: Content::Text("助手回答".to_string()),
            },
        ];

        strip_stale_workspace_contexts(&mut messages);
        assert_eq!(
            messages[0].content,
            Content::Text("用户原本的问题".to_string())
        );
    }

    #[test]
    fn test_inject_fresh_workspace_context() {
        let snapshot = format!("{}\nRoot: /test\nGit: clean{}", CONTEXT_TAG_START, CONTEXT_TAG_END);
        let mut messages = vec![
            Message {
                role: Role::User,
                content: Content::Text("第一个问题".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: Content::Text("回答一".to_string()),
            },
            Message {
                role: Role::User,
                content: Content::Text("第二个问题".to_string()),
            },
        ];

        inject_fresh_workspace_context(&mut messages, &snapshot);

        // 验证仅注入最后一条（最新轮次）User 消息
        assert_eq!(messages[0].content, Content::Text("第一个问题".to_string()));
        let last_text = match &messages[2].content {
            Content::Text(t) => t.as_str(),
            _ => panic!("Expected text"),
        };
        assert!(last_text.starts_with(CONTEXT_TAG_START));
        assert!(last_text.ends_with("第二个问题"));
    }

    #[test]
    fn test_normalize_role_alternation() {
        let messages = vec![
            Message {
                role: Role::User,
                content: Content::Text("消息一".to_string()),
            },
            Message {
                role: Role::User,
                content: Content::Text("消息二".to_string()),
            },
            Message {
                role: Role::Assistant,
                content: Content::Text("助手回复".to_string()),
            },
        ];

        let normalized = normalize_role_alternation(messages);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].role, Role::User);
        assert_eq!(normalized[1].role, Role::Assistant);
    }

    #[test]
    fn test_build_workspace_snapshot_detects_self_repo() {
        let cargo_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let snapshot = build_workspace_snapshot(cargo_dir);
        assert!(snapshot.contains(CONTEXT_TAG_START));
        assert!(snapshot.contains("Cargo.toml"));
        assert!(snapshot.contains(CONTEXT_TAG_END));
    }
}
