use std::path::PathBuf;

use tauri::AppHandle;

use crate::llm::services::agent_bundles;
use crate::llm::services::skills::list_enabled_skill_summaries_with_app;
use crate::llm::types::AgentMode;

// 系统提示文件名（相对工程目录 src/prompt）
const SYSTEM_PROMPT_FILE_NAME: &str = "system_prompt.md";

// 计划模式附加内容：当 agent_mode=plan 时合并到系统提示中。
// 该段与正常模式分离方便 semantics 清晰、可测。
const PLAN_MODE_SECTION: &str = r#"

## Plan Mode
- You are currently in plan mode.
- In this mode, prioritize understanding the problem, exploring the codebase, identifying constraints, and proposing a concrete implementation strategy.
- Do not edit files or run implementation tools before explicit user approval.
- When your plan is ready, call `exit_plan_mode` and pass the full final plan in its required `plan` argument (Markdown: title, context/background, goal, numbered implementation steps, verification notes). The plan is saved automatically and shown to the user as a structured panel — you do not need to manage plan files yourself.
- Use `ask_user_question` for extra clarifications only when needed to unblock planning decisions.
"#;

// 自动迭代模式附加内容：鼓励在单轮中自主推进，只有被真实阻塞时再请求用户输入。
const AUTO_MODE_SECTION: &str = r#"

## Auto Iteration Mode
- You are currently in auto iteration mode.
- Drive the task forward proactively with focused tool usage and iterative verification.
- Keep iterating until the task is meaningfully complete, then present a concise outcome.
- Tool permissions are fully auto-approved in this mode: do not wait for user approval on bash, file writes, or other tools.
- Ask for user input only when blocked by missing requirements or truly irreversible decisions that need human judgment.
"#;

const GLOBAL_MEMORY_SECTION: &str = r#"

## Memory
- You SHOULD call the `memory` tool to persist stable cross-session facts when:
  1. The user expresses a preference or correction (e.g. "太长了"、"不要兜底"、"用中文"、"我说的是...")
  2. The user reveals durable facts about themselves or their project
  3. The user establishes a workflow rule or convention
- Priority: user preferences and corrections > project facts > procedural details
- Write DECLARATIVE facts, not imperatives:
  ✓ "User prefers concise responses"
  ✗ "Always respond concisely"
- Do NOT store: secrets, credentials, private tokens, one-off tasks, environment errors, transient failures
- Keep entries concise, specific, and reusable
- Use `action="replace"` or `action="remove"` to consolidate or prune stale entries
"#;

fn read_non_empty_file(path: &PathBuf) -> Option<String> {
    // 读取文件文本，读取失败返回 None。
    let text = std::fs::read_to_string(path).ok()?;
    // 去掉首尾空白后判断是否为空。
    let trimmed = text.trim();
    // 空文件或全空白文件视为无效。
    if trimmed.is_empty() {
        return None;
    }
    // 返回裁剪后的新字符串。
    Some(trimmed.to_string())
}

fn main_prompt_path() -> PathBuf {
    // 从编译时清单目录开始构造绝对路径。
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        // 进入 src 目录。
        .join("src")
        // 进入 prompt 子目录。
        .join("prompt")
        // 拼接系统提示文件名。
        .join(SYSTEM_PROMPT_FILE_NAME)
}

/// 拼接指定锚点的插件提示词片段（启用插件的 promptSection 贡献，只增不改）。
/// 插件是宿主级安装的，与智能体套件无隶属关系——bundle 完整替换提示词时同样注入。
fn append_plugin_prompt_sections(
    prompt: String,
    app: &AppHandle,
    placement: &str,
) -> String {
    let sections = crate::llm::services::plugins::plugin_prompt_sections(app);
    let matched: Vec<_> = sections
        .into_iter()
        .filter(|(_, _, anchor)| anchor == placement)
        .collect();
    if matched.is_empty() {
        return prompt;
    }
    let mut result = prompt;
    for (plugin_name, content, _) in matched {
        result = format!(
            "{}\n\n## Plugin: {}\n{}\n",
            result, plugin_name, content
        );
    }
    result
}

pub fn load_system_prompt(
    app: &AppHandle,
    agent_mode: AgentMode,
    conversation_id: Option<&str>,
) -> Result<String, String> {
    // 子代理会话：使用专属精简提示词，完全跳过主工程协议、
    // bundle/skills/memory/plugin 段（对应工具对子代理不可见，写了就是误导）。
    // 工作区按父会话解析（子 ID 不在会话表里）。
    if crate::llm::services::subagent::is_subagent_conversation(conversation_id) {
        let parent = conversation_id
            .map(crate::llm::services::subagent::parent_conversation_id)
            .unwrap_or_default();
        let ws = crate::command::workspace::workspace_root_for_conversation(app, Some(parent))?;
        let prompt = crate::llm::services::subagent::system_prompt().replace(
            "Workspace root is provided in the first message.",
            &format!("Workspace root: {}.", ws.display()),
        );
        return Ok(prompt);
    }

    let bundle = agent_bundles::active_bundle(app, conversation_id);

    // 基础提示词：挂载了带提示词的智能体套件时**完整替换**默认系统提示词——
    // 用户定义的是独立智能体，不叠加 Nova 默认的工程协议；为空时回落默认 system_prompt.md。
    let prompt = match &bundle {
        Some(b) if !b.prompt.trim().is_empty() => b.prompt.trim().to_string(),
        _ => {
            // 计算系统提示文件路径。
            let path = main_prompt_path();
            // 读取并校验主提示词文件，失败时拒绝 fallback。
            read_non_empty_file(&path).ok_or_else(|| {
                format!(
                    "System prompt file is missing or empty: {}. Refusing to use fallback.",
                    path.display()
                )
            })?
        }
    };

    // 将 workspace 路径注入提示词（bundle 提示词同样支持占位符）。
    let ws = crate::command::workspace::workspace_root_for_conversation(app, conversation_id)?;
    let prompt = prompt.replace("{{NOVA_WORKSPACE}}", &ws.display().to_string());

    // 将平台信息注入提示词。
    let platform = if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    };
    let prompt = prompt.replace("{{NOVA_PLATFORM}}", platform);

    // 将 rg 完整路径注入提示词。复用 GrepTool 的 find_rg_path,
    // 保证提示词里写的路径与 Grep 实际使用的路径一致(含 env/bundled/PATH 回退)。
    let rg_path = crate::llm::tools::grep_tool::find_rg_path(app);
    let prompt = prompt.replace("{{RG_PATH}}", &rg_path);

    // 插件提示词片段（after-tools 锚点）：主提示词之后、Memory 段之前。
    let prompt = append_plugin_prompt_sections(prompt, app, "after-tools");

    // Memory 使用说明只有 memory 工具对当前智能体可见时才注入
    //（bundle 自定义工具清单排除了 memory 时，写它就是误导）。
    let memory_tool_visible = match &bundle {
        Some(b) => b.is_tool_enabled("memory"),
        None => true,
    };
    let prompt_with_memory = if memory_tool_visible {
        format!("{}{}", prompt, GLOBAL_MEMORY_SECTION)
    } else {
        prompt
    };

    // 注入全局记忆 snapshot：随记忆增删改实时重建，删除记忆后立即不再注入。
    let prompt_with_memory = match crate::llm::services::memory_dir::snapshot(app) {
        Some(snapshot_block) => format!("{}\n\n{}\n", prompt_with_memory, snapshot_block),
        None => prompt_with_memory,
    };

    // 插件提示词片段（before-memory 锚点）：记忆快照之后、Skills 段之前。
    let prompt_with_memory =
        append_plugin_prompt_sections(prompt_with_memory, app, "before-memory");

    // 注入可用 skill 元数据，AI 无需先 list 即可直接 run。
    // 已停用（全局设置）、不在当前 bundle 白名单内、或 Skill 工具本身被 bundle
    // 排除时（列出来模型也调不了），一律不注入。
    let skill_tool_visible = match &bundle {
        Some(b) => b.is_tool_enabled("Skill"),
        None => true,
    };
    let prompt_with_memory = if skill_tool_visible {
        match list_enabled_skill_summaries_with_app(app) {
            Ok(skills) if !skills.is_empty() => {
                let visible: Vec<_> = match &bundle {
                    Some(bundle) => skills
                        .into_iter()
                        .filter(|s| bundle.is_skill_enabled(&s.name))
                        .collect(),
                    None => skills,
                };
                if visible.is_empty() {
                    prompt_with_memory
                } else {
                    let lines: String = visible
                        .iter()
                        .map(|s| format!("- **{}**: {}", s.name, s.description))
                        .collect::<Vec<String>>()
                        .join("\n");
                    format!("{}\n\n## Available Skills\n{}\n", prompt_with_memory, lines)
                }
            }
            _ => prompt_with_memory,
        }
    } else {
        prompt_with_memory
    };

    // 插件提示词片段（end 锚点）：Skills 段之后、模式段之前。
    let prompt_with_memory = append_plugin_prompt_sections(prompt_with_memory, app, "end");

    // 按执行模式拼接附加段。
    match agent_mode {
        AgentMode::Plan => Ok(format!("{}{}", prompt_with_memory, PLAN_MODE_SECTION)),
        AgentMode::Auto => Ok(format!("{}{}", prompt_with_memory, AUTO_MODE_SECTION)),
        AgentMode::Agent => Ok(prompt_with_memory),
    }
}
