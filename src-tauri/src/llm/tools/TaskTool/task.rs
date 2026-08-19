use crate::llm::tools::{
    app_tool, AppExecuteFuture, ToolFailure, ToolOutcome, ToolRegistration,
};
use crate::llm::types::Tool;
use serde_json::{json, Value};
use tauri::AppHandle;

pub(super) fn registration() -> ToolRegistration {
    // read_only=true：子代理本身只做只读探索，进入批量并发执行器后
    // 同一轮的多个 Task 可并行运行（内部信号量再限同时 3 个）。
    app_tool(tool, execute_with_app_boxed, true, None)
}

pub fn tool() -> Tool {
    Tool {
        name: "Task".into(),
        description: r#"Launches a read-only research subagent that explores the codebase (or the web) in its own separate context and returns a final report. The subagent's intermediate steps (searches, file reads) do NOT consume this conversation's context — only its final report comes back.

## When to use this tool
- Investigations that require reading many files: "where is X implemented", "which modules depend on Y", "summarize how Z works".
- Answering questions whose supporting evidence would flood this context (dozens of search hits / large files).
- Independent research subtasks you can delegate while you keep working on the main task.
- NOT for: single quick lookups (do those yourself with Grep/Read), or anything requiring writes (the subagent is strictly read-only: Read/Grep/Glob/GitDiff/WebSearch/WebFetch only — no shell, no editing, no MCP, no plugins).

## How to use it
- `task` must be fully self-contained: the subagent sees ONLY this text (plus the workspace path). Include the goal, what to investigate, relevant paths/symbols you already know, and what the report should contain.
- Do not reference "the conversation above" or earlier tool results — summarize them into the task text instead.
- Multiple independent Task calls in the same turn run in parallel; prefer one batched call over sequential ones when the subtasks are independent.
- The subagent cannot ask you or the user questions; if info might be missing, say in the task what to assume or how to proceed.

## Return value
The tool result is the subagent's final report (conclusions + file/line references). Treat it as verified findings from this workspace."#
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Complete, self-contained instructions for the subagent: goal, scope, known context, expected report contents."
                },
                "description": {
                    "type": "string",
                    "description": "Short (3-5 word) summary of what this subagent investigates, shown to the user."
                }
            },
            "required": ["task"]
        }),
    }
}

fn execute_with_app_boxed(
    app: AppHandle,
    conversation_id: Option<String>,
    input: Value,
) -> AppExecuteFuture {
    Box::pin(async move {
        execute_async(&app, conversation_id.as_deref(), input).await
    })
}

async fn execute_async(
    app: &AppHandle,
    conversation_id: Option<&str>,
    input: Value,
) -> Result<ToolOutcome, ToolFailure> {
    let task_text = input
        .get("task")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ToolFailure::invalid_input("Missing required parameter: task"))?;

    // Task 必须挂在真实会话下（派生子 ID、记账、取消传播都需要父 ID）。
    let parent = conversation_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            ToolFailure::invalid_input("Task tool requires an active conversation")
        })?;

    let report = crate::llm::services::subagent::run(app, parent, task_text)
        .await
        .map_err(ToolFailure::new)?;

    Ok(ToolOutcome::text(report))
}
