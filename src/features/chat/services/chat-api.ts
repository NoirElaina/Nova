import { invoke } from "@tauri-apps/api/core";
import type {
  AgentMode,
  ChatMessage,
  ConversationMemory,
  ConversationMeta,
  PersistedMessage,
  ScheduledTask,
  ToolExecutionEntry,
} from "../../../lib/chat-types";
import type { PermissionActionName } from "../../../lib/chat-payloads";
import { buildConversationTitle } from "../utils/session-memory";

type ChatRequestContent = string | Array<Record<string, unknown>>;

type ChatRequestMessage = Pick<ChatMessage, "role"> & {
  content: ChatRequestContent;
};

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

export type RagUploadDocumentInput = {
  sourceName: string;
  sourceType: string;
  mimeType?: string;
  content: string;
};

export type RagRejectedItem = {
  sourceName: string;
  reason: string;
};

export type RagUpsertResult = {
  added: number;
  updated: number;
  rejected: RagRejectedItem[];
  totalDocuments: number;
  totalChars: number;
};

export type RagDocumentMeta = {
  id: string;
  sourceName: string;
  sourceType: string;
  mimeType?: string;
  contentChars: number;
  preview: string;
  checksum: string;
  createdAt: number;
  updatedAt: number;
};

export type RagDocumentContent = RagDocumentMeta & {
  content: string;
};

export type ShellSessionStatus = {
  exists: boolean;
  alive: boolean;
  busy: boolean;
  cwd?: string | null;
  backgroundPids: number[];
  backgroundCount: number;
};

export type ShellCommandResult = {
  stdout: string;
  stderr: string;
  exitCode?: number | null;
  cwd?: string | null;
  timedOut: boolean;
  cancelled: boolean;
  background: boolean;
  pid?: number | null;
};

export type UserTerminalInfo = {
  conversationId?: string | null;
  sessionId: string;
  cwd: string;
};

export type UserTerminalOutputEvent = {
  conversationId?: string | null;
  sessionId: string;
  kind: "output" | "error" | "exit";
  data?: string | null;
  exitCode?: number | null;
  error?: string | null;
};

export type LiveChatTurnStatus = {
  conversationId: string;
  state: "running" | "completed" | "needs_user_input" | "cancelled" | "stop_hook_prevented" | "error";
  assistantResponse: string;
  assistantReasoning: string;
  startedAt: number;
  updatedAt: number;
};

export type FileDiffLine = {
  kind: "context" | "add" | "remove";
  oldLine?: number | null;
  newLine?: number | null;
  text: string;
};

export type WorkspaceFileChange = {
  path: string;
  absolutePath: string;
  changeType: "added" | "deleted" | "modified";
  additions: number;
  deletions: number;
  diff: FileDiffLine[];
};

export type WorkspaceDiff = {
  files: WorkspaceFileChange[];
  totalAdditions: number;
  totalDeletions: number;
};

export async function listConversations(): Promise<ConversationMeta[]> {
  const items = await invoke<ConversationMeta[]>("list_conversations");
  return items || [];
}

export async function setConversationPinned(
  conversationId: string,
  pinned: boolean,
): Promise<void> {
  await invoke("set_conversation_pinned", {
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
  return invoke<string>("export_conversation", {
    conversationId,
    format,
  });
}

export async function exportRenderedConversationPdf(
  conversationId: string,
  title: string,
  html: string,
): Promise<string> {
  return invoke<string>("export_rendered_conversation_pdf", {
    conversationId,
    title,
    html,
  });
}

export async function createConversation(
  seedTitle?: string,
  workspacePath?: string,
): Promise<ConversationMeta> {
  return invoke<ConversationMeta>("create_conversation", {
    title: seedTitle?.trim() ? buildConversationTitle(seedTitle) : undefined,
    workspacePath: workspacePath?.trim() ? workspacePath.trim() : undefined,
  });
}

export async function loadConversationHistory(conversationId: string): Promise<PersistedMessage[]> {
  const saved = await invoke<PersistedMessage[]>("load_history", { conversationId });
  return saved || [];
}

export async function appendConversationMessage(
  conversationId: string,
  message: ChatMessage,
): Promise<void> {
  await invoke("append_history", { conversationId, message });
}

export async function replaceConversationHistory(
  conversationId: string,
  messages: ChatMessage[],
): Promise<void> {
  await invoke("replace_history", { conversationId, messages });
}

export async function loadConversationToolLogs(
  conversationId: string,
): Promise<ToolExecutionEntry[]> {
  const logs = await invoke<ToolExecutionEntry[]>("load_conversation_tool_logs", {
    conversationId,
  });
  return logs || [];
}

export async function upsertConversationToolLog(
  conversationId: string,
  log: ToolExecutionEntry,
): Promise<void> {
  await invoke("upsert_conversation_tool_log", {
    conversationId,
    log,
  });
}

export async function getConversationMemory(
  conversationId: string,
): Promise<ConversationMemory | null> {
  return invoke<ConversationMemory | null>("get_conversation_memory", { conversationId });
}

export async function upsertConversationMemory(
  conversationId: string,
  summary: string,
  keyFacts: string[],
): Promise<void> {
  await invoke("upsert_conversation_memory", {
    conversationId,
    summary,
    keyFacts,
  });
}

export async function deleteConversation(conversationId: string): Promise<void> {
  await invoke("delete_conversation", { conversationId });
}

export async function getShellSessionStatus(
  conversationId: string | null,
): Promise<ShellSessionStatus> {
  return invoke<ShellSessionStatus>("get_shell_session_status", {
    conversationId,
  });
}

export async function executeShellCommandForConversation(
  conversationId: string | null,
  command: string,
  options?: {
    timeoutMs?: number;
    background?: boolean;
  },
): Promise<ShellCommandResult> {
  return invoke<ShellCommandResult>("execute_shell_command_for_conversation", {
    conversationId,
    command,
    timeoutMs: options?.timeoutMs,
    background: options?.background,
  });
}

export async function startUserTerminal(
  conversationId: string | null,
  size: { rows?: number; cols?: number } = {},
): Promise<UserTerminalInfo> {
  return invoke<UserTerminalInfo>("user_terminal_start", {
    conversationId,
    rows: size.rows,
    cols: size.cols,
  });
}

export async function writeUserTerminal(
  conversationId: string | null,
  data: string,
): Promise<void> {
  await invoke("user_terminal_write", {
    conversationId,
    data,
  });
}

export async function resizeUserTerminal(
  conversationId: string | null,
  size: { rows?: number; cols?: number },
): Promise<void> {
  await invoke("user_terminal_resize", {
    conversationId,
    rows: size.rows,
    cols: size.cols,
  });
}

export async function stopUserTerminal(conversationId: string | null): Promise<void> {
  await invoke("user_terminal_stop", {
    conversationId,
  });
}

export async function getWorkspaceDiff(
  conversationId: string | null,
): Promise<WorkspaceDiff> {
  return invoke<WorkspaceDiff>("get_workspace_diff", {
    conversationId,
  });
}

export type GitRepoStatus = {
  initialized: boolean;
  path: string;
  branch?: string | null;
  worktree?: string | null;
};

export type InitGitRepoResult = {
  /** true 表示这次调用新建了 `.git`；false 表示仓库已存在，本次为空操作。 */
  created: boolean;
  path: string;
};

/** 查询会话工作区的 git 初始化状态。 */
export async function getGitRepoStatus(
  conversationId: string | null,
): Promise<GitRepoStatus> {
  return invoke<GitRepoStatus>("get_git_repo_status", { conversationId });
}

/** 基于显式工作区路径查询 git 状态。供 EnvironmentBar 在无会话时使用。 */
export async function getWorkspaceGitStatus(
  workspacePath: string,
): Promise<GitRepoStatus> {
  return invoke<GitRepoStatus>("get_workspace_git_status", { workspacePath });
}

/**
 * 显式把会话工作区初始化为 git 仓库。
 * 默认流程不再自动 `git init`，必须由用户在审查页点击按钮触发，避免污染用户工作目录。
 */
export async function initGitRepo(
  conversationId: string | null,
): Promise<InitGitRepoResult> {
  return invoke<InitGitRepoResult>("init_git_repo", { conversationId });
}

export async function sendChatMessage(
  conversationId: string | null,
  messages: ChatRequestMessage[],
  planMode: boolean,
  agentMode: AgentMode,
): Promise<void> {
  await invoke("send_chat_message", {
    conversationId,
    messages,
    planMode,
    agentMode,
  });
}

export async function getChatTurnStatus(
  conversationId: string | null,
): Promise<LiveChatTurnStatus | null> {
  return invoke<LiveChatTurnStatus | null>("get_chat_turn_status", {
    conversationId,
  });
}

export async function ackChatTurnStatus(conversationId: string | null): Promise<boolean> {
  return invoke<boolean>("ack_chat_turn_status", {
    conversationId,
  });
}

export async function upsertConversationRagDocuments(
  conversationId: string,
  documents: RagUploadDocumentInput[],
): Promise<RagUpsertResult> {
  return invoke<RagUpsertResult>("rag_upsert_conversation_documents", {
    conversationId,
    documents,
  });
}

export async function listConversationRagDocuments(
  conversationId: string,
): Promise<RagDocumentMeta[]> {
  return invoke<RagDocumentMeta[]>("rag_list_conversation_documents", {
    conversationId,
  });
}

export async function readRagDocument(
  documentId: string,
  conversationId?: string | null,
): Promise<RagDocumentContent | null> {
  return invoke<RagDocumentContent | null>("rag_read_document", {
    documentId,
    conversationId,
  });
}

export async function getActiveModelRuntime(): Promise<ActiveModelRuntime> {
  const settings = await invoke<RuntimeSettings>("get_settings");
  const provider = (settings.provider || "anthropic").trim().toLowerCase() || "anthropic";
  const profile = settings.providerProfiles?.[provider] ?? {};
  const protocol = (profile.protocol || (provider.includes("anthropic") ? "anthropic" : "openai"))
    .trim()
    .toLowerCase();
  const model = (profile.model || "").trim();
  const windowTokens = model
    ? await invoke<number>("get_model_window_tokens", { model })
    : 200_000;

  return {
    provider,
    protocol,
    model,
    windowTokens,
  };
}

export async function estimateTextTokens(
  text: string,
  protocol = "anthropic",
): Promise<number> {
  return invoke<number>("estimate_text_tokens", { text, protocol });
}

export async function cancelChatMessage(conversationId: string | null): Promise<boolean> {
  return invoke<boolean>("cancel_chat_message", {
    conversationId,
  });
}

export async function submitPermissionDecision(
  conversationId: string | null,
  requestId: string,
  action: PermissionActionName,
): Promise<boolean> {
  return invoke<boolean>("submit_permission_decision", {
    conversationId,
    requestId,
    action,
  });
}

export async function listScheduledTasks(): Promise<ScheduledTask[]> {
  const tasks = await invoke<ScheduledTask[]>("list_scheduled_tasks");
  return tasks || [];
}

export async function createScheduledTask(payload: {
  cron: string;
  prompt: string;
  recurring?: boolean;
  durable?: boolean;
}): Promise<ScheduledTask> {
  return invoke<ScheduledTask>("create_scheduled_task", payload);
}

export async function deleteScheduledTask(id: string): Promise<boolean> {
  return invoke<boolean>("delete_scheduled_task", { id });
}
