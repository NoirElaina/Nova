import { ipcInvoke, newIdempotencyKey } from "../../../lib/ipc";
import type {
  ChatAttachment,
  ChatMessage,
  ConversationMeta,
  ConversationUsageSummary,
  PersistedMessage,
  ScheduledTask,
  ToolExecutionEntry,
  TurnTrace,
} from "../../../lib/chat-types";
import type { PermissionActionName } from "../../../lib/chat-payloads";
import { buildConversationTitle } from "../utils/session-memory";

type RuntimeProviderProfile = {
  protocol?: string;
  model?: string;
};

type RuntimeSettings = {
  provider?: string;
  providerProfiles?: Record<string, RuntimeProviderProfile>;
};

export type ActiveModelRuntime = {
  provider: string;
  protocol: string;
  model: string;
  windowTokens: number;
};

import type {
  ConversationPlan,
  FileDiffLine,
  GitRepoStatus,
  InitGitRepoResult,
  SandboxStatus,
  SessionFileMeta,
  WorkspaceDiff,
  WorkspaceFileChange,
} from "../../../lib/protocol";
export type {
  ConversationPlan,
  FileDiffLine,
  GitRepoStatus,
  InitGitRepoResult,
  SandboxStatus,
  SessionFileMeta,
  WorkspaceDiff,
  WorkspaceFileChange,
};

export type UserTerminalInfo = {
  conversationId?: string | null;
  terminalId?: string;
  sessionId: string;
  shellName?: string;
  cwd: string;
};

export type UserTerminalOutputEvent = {
  conversationId?: string | null;
  terminalId?: string;
  sessionId: string;
  kind: "output" | "error" | "exit";
  data?: string | null;
  exitCode?: number | null;
  error?: string | null;
};

export async function listConversations(): Promise<ConversationMeta[]> {
  const items = await ipcInvoke<ConversationMeta[]>("list_conversations");
  return items || [];
}

export async function getConversationUsage(
  conversationId: string,
): Promise<ConversationUsageSummary> {
  return ipcInvoke<ConversationUsageSummary>("get_conversation_usage", { conversationId });
}

export async function setConversationPinned(
  conversationId: string,
  pinned: boolean,
): Promise<void> {
  await ipcInvoke("set_conversation_pinned", {
    conversationId,
    pinned,
  });
}

export type ConversationExportFormat = "json" | "pdf";
type JsonConversationExportFormat = Extract<ConversationExportFormat, "json">;

export async function exportConversation(
  conversationId: string,
  format: JsonConversationExportFormat,
): Promise<string> {
  return ipcInvoke<string>("export_conversation", {
    conversationId,
    format,
  });
}

export async function exportRenderedConversationPdf(
  conversationId: string,
  title: string,
  html: string,
): Promise<string> {
  return ipcInvoke<string>("export_rendered_conversation_pdf", {
    conversationId,
    title,
    html,
  });
}

export async function createConversation(
  seedTitle?: string,
  workspacePath?: string,
  idempotencyKey: string = newIdempotencyKey(),
): Promise<ConversationMeta> {
  return ipcInvoke<ConversationMeta>("create_conversation", {
    title: seedTitle?.trim() ? buildConversationTitle(seedTitle) : undefined,
    workspacePath: workspacePath?.trim() ? workspacePath.trim() : undefined,
    idempotencyKey,
  });
}

export async function loadConversationHistory(conversationId: string): Promise<PersistedMessage[]> {
  const saved = await ipcInvoke<PersistedMessage[]>("load_history", { conversationId });
  return saved || [];
}

/** 后端 HistoryMessage.id 是 Option<i64>；前端临时 id 是 "user-..." 字符串，不能原样下发。 */
function toHistoryMessagePayload(message: ChatMessage): Record<string, unknown> {
  const payload: Record<string, unknown> = {
    role: message.role,
    content: message.content,
  };
  if (message.reasoning != null && message.reasoning.trim()) {
    payload.reasoning = message.reasoning;
  }
  if (message.attachments != null && message.attachments.length > 0) {
    payload.attachments = message.attachments;
  }
  if (typeof message.tokenUsage === "number") {
    payload.tokenUsage = message.tokenUsage;
  }
  if (message.cost != null) {
    payload.cost = message.cost;
  }
  // 仅当 id 已是数据库数字主键时才带上；临时 client id 一律省略
  const rawId = message.id;
  if (rawId != null && /^\d+$/.test(String(rawId).trim())) {
    const n = Number.parseInt(String(rawId).trim(), 10);
    if (Number.isFinite(n) && n > 0) {
      payload.id = n;
    }
  }
  return payload;
}

export async function replaceConversationHistory(
  conversationId: string,
  messages: ChatMessage[],
): Promise<void> {
  await ipcInvoke("replace_history", {
    conversationId,
    messages: messages.map(toHistoryMessagePayload),
  });
}

export async function loadConversationToolLogs(
  conversationId: string,
): Promise<ToolExecutionEntry[]> {
  const logs = await ipcInvoke<ToolExecutionEntry[]>("load_conversation_tool_logs", {
    conversationId,
  });
  return logs || [];
}

/** 加载会话的回合轨迹（调试面板：按回合分组的完整交互记录）。 */
export async function getConversationTurnTraces(
  conversationId: string,
): Promise<TurnTrace[]> {
  const traces = await ipcInvoke<TurnTrace[]>("get_conversation_turn_traces", {
    conversationId,
  });
  return traces || [];
}

export async function deleteConversation(conversationId: string): Promise<void> {
  await ipcInvoke("delete_conversation", { conversationId });
}

export async function startUserTerminal(
  conversationId: string | null,
  size: { rows?: number; cols?: number } = {},
  terminalId?: string | null,
): Promise<UserTerminalInfo> {
  return ipcInvoke<UserTerminalInfo>("user_terminal_start", {
    conversationId,
    terminalId: terminalId ?? null,
    rows: size.rows,
    cols: size.cols,
  });
}

export async function writeUserTerminal(
  conversationId: string | null,
  data: string,
  terminalId?: string | null,
): Promise<void> {
  await ipcInvoke("user_terminal_write", {
    conversationId,
    terminalId: terminalId ?? null,
    data,
  });
}

export async function resizeUserTerminal(
  conversationId: string | null,
  size: { rows?: number; cols?: number },
  terminalId?: string | null,
): Promise<void> {
  await ipcInvoke("user_terminal_resize", {
    conversationId,
    terminalId: terminalId ?? null,
    rows: size.rows,
    cols: size.cols,
  });
}

export async function stopUserTerminal(
  conversationId: string | null,
  terminalId?: string | null,
): Promise<void> {
  await ipcInvoke("user_terminal_stop", {
    conversationId,
    terminalId: terminalId ?? null,
  });
}

export async function getWorkspaceDiff(
  conversationId: string | null,
): Promise<WorkspaceDiff> {
  return ipcInvoke<WorkspaceDiff>("get_workspace_diff", {
    conversationId,
  });
}

/** 查询会话工作区的 git 初始化状态。 */
export async function getGitRepoStatus(
  conversationId: string | null,
): Promise<GitRepoStatus> {
  return ipcInvoke<GitRepoStatus>("get_git_repo_status", { conversationId });
}

/** 基于显式工作区路径查询 git 状态。供 EnvironmentBar 在无会话时使用。 */
export async function getWorkspaceGitStatus(
  workspacePath: string,
): Promise<GitRepoStatus> {
  return ipcInvoke<GitRepoStatus>("get_workspace_git_status", { workspacePath });
}

/**
 * 显式把会话工作区初始化为 git 仓库。
 * 默认流程不再自动 `git init`，必须由用户在审查页点击按钮触发，避免污染用户工作目录。
 */
export async function initGitRepo(
  conversationId: string | null,
): Promise<InitGitRepoResult> {
  return ipcInvoke<InitGitRepoResult>("init_git_repo", { conversationId });
}

/** 发送给全新现代化 Agent 引擎 (单一真实信源 + ReAct 自愈验证) */
export async function sendModernAgentTurn(
  conversationId: string,
  prompt: string,
  attachments?: ChatAttachment[],
  mode?: import("@/lib/protocol").TurnSubmitMode,
  idempotencyKey: string = newIdempotencyKey(),
): Promise<import("@/lib/protocol").AgentTurnSubmitOutcome> {
  return ipcInvoke<import("@/lib/protocol").AgentTurnSubmitOutcome>("send_modern_agent_turn", {
    conversationId,
    prompt,
    attachments: attachments && attachments.length > 0 ? attachments : null,
    mode: mode ?? null,
    idempotencyKey,
  });
}

/** 追加一条纯文本消息到会话事件日志（分支转存等场景）。 */
export async function appendPlainChatMessage(
  conversationId: string,
  role: "user" | "assistant",
  content: string,
): Promise<void> {
  await ipcInvoke("append_plain_chat_message", { conversationId, role, content });
}



export async function saveSessionFile(
  conversationId: string,
  filename: string,
  content: string | null,
  rawBytes: number[] | null,
): Promise<SessionFileMeta> {
  return ipcInvoke<SessionFileMeta>("save_session_file", {
    conversationId,
    filename,
    content,
    rawBytes,
  });
}

export async function listSessionFiles(
  conversationId: string,
): Promise<SessionFileMeta[]> {
  return ipcInvoke<SessionFileMeta[]>("list_session_files", {
    conversationId,
  });
}

export async function readSessionFile(
  conversationId: string,
  filename: string,
): Promise<string> {
  return ipcInvoke<string>("read_session_file", {
    conversationId,
    filename,
  });
}

/** 读取会话 plan；会话还没有 plan 时返回 null。 */
export async function getConversationPlan(
  conversationId: string | null,
): Promise<ConversationPlan | null> {
  return ipcInvoke<ConversationPlan | null>("get_conversation_plan", {
    conversationId,
  });
}

export async function getRuntimeSettings(): Promise<RuntimeSettings> {
  const settings = await ipcInvoke<RuntimeSettings>("get_settings");
  return settings || {};
}

export async function getActiveModelRuntime(): Promise<ActiveModelRuntime> {
  const settings = await ipcInvoke<RuntimeSettings>("get_settings");
  const provider = (settings.provider || "anthropic").trim().toLowerCase() || "anthropic";
  const profile = settings.providerProfiles?.[provider] ?? {};
  const protocol = (profile.protocol || (provider.includes("anthropic") ? "anthropic" : "openai"))
    .trim()
    .toLowerCase();
  const model = (profile.model || "").trim();
  const windowTokens = model
    ? await ipcInvoke<number>("get_model_window_tokens", { model })
    : 200_000;

  return {
    provider,
    protocol,
    model,
    windowTokens,
  };
}

export async function estimateTextTokens(text: string): Promise<number> {
  return ipcInvoke<number>("estimate_text_tokens", { text });
}

export async function cancelChatMessage(conversationId: string | null): Promise<boolean> {
  return ipcInvoke<boolean>("cancel_modern_agent_turn", {
    conversationId,
  });
}

export async function submitPermissionDecision(
  conversationId: string | null,
  requestId: string,
  action: PermissionActionName,
): Promise<boolean> {
  return ipcInvoke<boolean>("submit_permission_decision", {
    conversationId,
    requestId,
    action,
  });
}

export async function listScheduledTasks(): Promise<ScheduledTask[]> {
  const tasks = await ipcInvoke<ScheduledTask[]>("list_scheduled_tasks");
  return tasks || [];
}

export async function createScheduledTask(payload: {
  cron: string;
  prompt: string;
  recurring?: boolean;
  durable?: boolean;
}): Promise<ScheduledTask> {
  return ipcInvoke<ScheduledTask>("create_scheduled_task", payload);
}

export async function deleteScheduledTask(id: string): Promise<boolean> {
  return ipcInvoke<boolean>("delete_scheduled_task", { id });
}

export type CompactOutcome = {
  beforeTokens: number;
  afterTokens: number;
  savedTokens: number;
  summary: string;
};

export async function manualCompactConversation(
  conversationId: string,
): Promise<CompactOutcome> {
  return ipcInvoke<CompactOutcome>("manual_compact_conversation", { conversationId });
}

export type AgentNode = {
  id: string;
  parentId: string | null;
  role: string;
  nickname: string;
  task: string;
  status: "open" | "closed";
  createdAt: number;
  updatedAt: number;
  resultPreview: string | null;
};

export type BoardChannel = {
  name: string;
  description: string;
  createdAt: number;
  createdBy: string;
  messageCount: number;
};

export type BoardPost = {
  id: string;
  channel: string;
  threadId: string | null;
  authorId: string;
  authorNickname: string;
  authorRole: string;
  content: string;
  createdAt: number;
};

export async function getAgentGraph(conversationId: string): Promise<AgentNode[]> {
  const result = await ipcInvoke<AgentNode[]>("get_agent_graph", { conversationId });
  return result || [];
}

export async function getBoardChannels(conversationId: string): Promise<BoardChannel[]> {
  const result = await ipcInvoke<BoardChannel[]>("get_board_channels", { conversationId });
  return result || [];
}

export async function getBoardPosts(
  conversationId: string,
  channel?: string,
  threadId?: string,
  limit?: number,
): Promise<BoardPost[]> {
  const result = await ipcInvoke<BoardPost[]>("get_board_posts", {
    conversationId,
    channel,
    threadId,
    limit,
  });
  return result || [];
}

export async function postBoardMessage(
  conversationId: string,
  channel: string,
  content: string,
  threadId?: string,
  authorNickname?: string,
  authorRole?: string,
  idempotencyKey: string = newIdempotencyKey(),
): Promise<BoardPost> {
  return ipcInvoke<BoardPost>("post_board_message", {
    conversationId,
    channel,
    content,
    threadId,
    authorNickname,
    authorRole,
    idempotencyKey,
  });
}

export async function interruptAgent(
  conversationId: string,
  agentId: string,
): Promise<boolean> {
  return ipcInvoke<boolean>("interrupt_agent", {
    conversationId,
    agentId,
  });
}

export async function sendAgentMessage(
  conversationId: string,
  agentId: string,
  directive: string,
): Promise<boolean> {
  return ipcInvoke<boolean>("send_agent_message", {
    conversationId,
    agentId,
    directive,
  });
}

export async function getSandboxStatus(): Promise<SandboxStatus> {
  return ipcInvoke<SandboxStatus>("get_sandbox_status");
}

export async function exportDiagnosticBundle(targetPath?: string): Promise<string> {
  return ipcInvoke<string>("export_diagnostic_bundle", {
    targetPath: targetPath ?? null,
  });
}
