# Nova

Nova is a local desktop coding assistant for real project execution, controllable automation, and human-in-the-loop workflows.

## Core Features

- Conversation-scoped workspace root: each conversation has one persisted `WorkspaceRoot` used by the file tree, `NOVA_WORKSPACE`, and terminal reset.
- Tool-driven execution: runs shell commands, inspects output, and continues tasks end-to-end.
- Conversation-scoped persistent terminal sessions for `execute_bash` / `execute_powershell`, with command history visible in the Terminal workspace tab.
- Built-in Nova Browser automation: agents can open or focus the Browser tab, launch the independent browser window, navigate, snapshot, click, type, and reset pages.
- Browser annotation flow: users can select elements in the Nova Browser and add the captured page context back into the chat input.
- Workspace drawer: review agent output, token usage, session files, terminal activity, project files, and browser state without leaving the conversation.
- Modular tool registry: built-in tools self-describe registration, permissions, app execution, and post-processing so new tools can be mounted with one entry in `src-tauri/src/llm/tools/mod.rs`.
- Native LSP tools: diagnostics, definitions, references, symbols, hover, and language-server status are handled by Nova's backend instead of MCP.
- Multi-provider model access: switch providers and models with isolated per-provider profiles.
- Agent modes: `Agent`, `Plan`, and `Auto` modes are available from the input area.
- MCP connectivity: register MCP servers, inspect tools/resources, and invoke MCP tools through explicit tool names such as `mcp__server__tool`.
- Human clarification flow: pauses and asks focused questions when key details are missing.
- Approval controls: supports allow once, allow for session, and deny decisions for sensitive operations.
- Conversation continuity: restores trusted model context from turn snapshots, with compact context and memory updates for long-running sessions.
- File-backed cross-session memory: long-term memory is stored under the app data `memory/` directory, retrieved per request, and auto-maintained from stable user preferences and rules.
- Hook configuration: lifecycle, tool-flow, stop, subagent, session-end, and error hooks are configured through file-backed settings.
- Agent profiles: custom agent markdown profiles are stored in the app data directory and can be created, edited, and deleted from the Agent screen.
- Scheduled task automation: create/list/delete cron-based tasks with session or durable persistence.
- Optional app file logging: unified software logs can be enabled from settings; the default is off.

## Desktop Workspace

The right-side workspace drawer is the main companion panel for a conversation.

- Workspace: browse and open files from the current conversation's workspace root, resize or hide the file tree, switch that conversation's root, and view native LSP status/diagnostics.
- Review: inspect agent changes and execution context.
- Usage: view token and cost trends without altering chart animations.
- Files: inspect files uploaded into the current session.
- Terminal: watch AI shell commands in real time and run manual commands in the same persistent conversation shell; reset returns to that conversation's workspace root.
- Browser: keep browser status, annotation controls, and automation entry points while real page rendering runs in an independent Nova Browser window.

## Built-In Browser

Nova Browser is intentionally not rendered as an embedded page inside the main chat UI. It runs in an independent child window so WebView content does not float above settings modals, chat, or workspace panels.

- Agents can trigger `nova_browser_navigate`, `nova_browser_snapshot`, `nova_browser_click`, `nova_browser_type`, and `nova_browser_reset`.
- If the browser is not open, browser tools request the Browser workspace tab and window automatically.
- The Browser workspace tab shows current status and controls; it does not render the live page.
- Closing the browser clears the active page state for the conversation, so the next manual open starts clean instead of restoring the last URL.
- Browser annotation selection can package selected DOM context as a markdown upload for the next user message.

## Tool System

- Built-in tools are mounted through the central registry in `src-tauri/src/llm/tools/mod.rs`.
- Each tool module owns its own registration metadata instead of spreading behavior across global `match` branches.
- New tools can be scaffolded from `src-tauri/src/llm/tools/NewToolTemplate/`.
- MCP tools are exposed as explicit names like `mcp__playwright__browser_navigate` instead of a generic dispatcher tool.
- Shell tools use conversation-scoped persistent sessions, start in the conversation workspace root, and support foreground timeouts plus background processes.
- LSP tools are built in as `lsp_status`, `lsp_diagnostics`, `lsp_definition`, `lsp_references`, `lsp_symbols`, and `lsp_hover`; they do not require an MCP language-server bridge.

## Agent Turn Flow

- Turn orchestration lives in `src-tauri/src/llm/query.rs`; provider-specific request and stream handling lives under `src-tauri/src/llm/providers/`.
- Non-first turns restore model context from the saved turn snapshot. If a non-first turn has no snapshot, Nova fails fast instead of trusting frontend-rendered chat history as a fallback.
- Dynamic context, including session RAG, MCP server catalog, global memory, and hook-injected messages, is stripped before saving the turn snapshot and regenerated on the next turn.
- Providers normalize stream events into shared `Delta` values through `stream_runner.rs`.
- Persisted snapshots must keep tool pairs valid: every assistant `ToolUse` must have exactly one matching user `ToolResult`, and duplicate `ToolResult` blocks for the same `tool_use_id` are invalid for Anthropic.
- Cancellation keeps partial assistant output and closes only missing tool results with a synthetic `"Interrupted by user"` result. Existing tool results must not be duplicated.
- Tool and hook side-channel messages are part of the model context. For example, screenshot tools may remove large base64 payloads from textual tool results and attach images as additional context messages.

## Memory System

- Session memory supports handover, compact context, and conversation restore.
- Cross-session memory uses a file-backed memory directory instead of a database table.
- Memory records are grouped by kind: `preference`, `rule`, and `fact`.
- Retrieval is query-aware: Nova injects persistent rules/preferences plus relevant facts for the current request.
- New user preferences and rules can be auto-remembered from chat messages.
- Memory writes perform inline dedupe and conflict cleanup so newer rules replace stale duplicates instead of accumulating parallel variants.

## Scheduled Tasks

Nova includes a built-in scheduled task system for recurring or one-shot prompt execution.

- Cron format: 5 fields (minute hour day-of-month month day-of-week).
- Storage modes:
  - `session`: in-memory, cleared on app restart.
  - `durable`: persisted under `app_data_dir` in `scheduled_tasks.json`.
- Task conversation binding:
  - Every newly created task automatically creates and binds a dedicated conversation.
  - Triggered task content is written into the bound conversation.
  - The scheduler also launches an automatic model turn for the bound conversation.
- UI behavior:
  - Schedule screen shows task metadata including bound conversation id.
  - Each task has a "View Task Details" action that opens its bound conversation directly.
  - Task-bound scheduled conversations are hidden from the normal Recents list to reduce noise.
- One-shot reliability:
  - One-shot tasks attempt deletion after trigger.
  - If deletion fails, the per-minute trigger guard is retained so the same task does not retrigger repeatedly in that minute.

## Configuration Screens

- General: theme, language, and app file logging.
- Model: provider profiles, API settings, and custom model names.
- MCP: server registration and enable/disable controls.
- RAG, skills, memory, and data settings live under the settings modal.
- Agent, Scheduled Tasks, and Hooks are top-level sidebar screens with compact shadcn-vue based layouts.

## Interaction Flow

1. You give a task in chat.
2. Nova plans and executes using available tools.
3. If required information is missing, Nova asks a targeted question instead of guessing.
4. Nova resumes execution and returns a complete result.

## Development Commands

- Start web UI only: `npm run dev`
- Start Tauri desktop app: `npm run tauri`
- Build frontend bundle: `npm run build`
- Build desktop app: `npm run tauri:build`
- Check Rust backend: `cargo check --manifest-path src-tauri/Cargo.toml`
