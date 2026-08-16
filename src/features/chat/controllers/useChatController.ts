import { computed, onMounted, onUnmounted, ref, shallowRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { emitToast, NOVA_CHAT_ERROR_EVENT, type ChatErrorPayload } from "../../../lib/toast";
import {
  cancelChatMessage,
  submitPermissionDecision,
  type SessionFileMeta,
  upsertConversationToolLog,
} from "../services/chat-api";
import type {
  AgentMode,
  AssistantTranscriptSegment,
  ChatMessage,
  ChatMessageEvent,
  ContextCompactSummary,
  ConversationMemory,
  ConversationMeta,
  NeedsUserInputPayload,
  PendingUploadFile,
  ToolExecutionEntry,
  TurnCost,
  ContextUsage,
} from "../../../lib/chat-types";
import {
  type LiveTurnStage,
  type ChatScreenHandle,
  type ConversationTurnRuntimeState,
  type MainView,
  type ScheduledTaskTriggerEvent,
} from "./chat-controller-types";
import {
  bindActiveRuntimeState,
  resetPendingPromptState,
} from "./chat-runtime-state";
import { createConversationOperations } from "./chat-conversation-ops";
import { createChatStreamOperations } from "./chat-stream-ops";
import { createSendOperations } from "./chat-send-ops";

export function useChatController() {
  const messages = shallowRef<ChatMessage[]>([]);
  const isGenerating = ref(false);
  const currentStage = ref<LiveTurnStage>("processing");
  const assistantResponse = ref("");
  const assistantReasoning = ref("");
  const assistantSegments = ref<AssistantTranscriptSegment[]>([]);
  const assistantTokenUsage = ref<number | undefined>(undefined);
  const assistantTurnCost = ref<TurnCost | undefined>(undefined);
  const conversations = ref<ConversationMeta[]>([]);
  const activeConversationId = ref("");
  /** 当前工作区路径（前端状态）。空字符串表示使用后端默认工作区。 */
  const activeWorkspacePath = ref("");
  const conversationFiles = ref<SessionFileMeta[]>([]);
  const pendingUploads = ref<PendingUploadFile[]>([]);
  const pendingQuestion = ref<NeedsUserInputPayload | null>(null);
  const pendingPermissionRequestId = ref<string | null>(null);
  const conversationMemory = ref<ConversationMemory | null>(null);
  const mainView = ref<MainView>("chat");
  const currentToolStartedAt = ref<number | null>(null);
  const currentToolCalls = ref(0);
  const currentToolDurationMs = ref(0);
  const currentContextUsage = ref<ContextUsage | undefined>(undefined);
  const currentContextCompacts = ref<ContextCompactSummary[]>([]);
  const currentContextTokens = ref(0);
  const currentInputTokens = ref(0);
  const currentOutputTokens = ref(0);
  const currentTurnId = ref<string | null>(null);
  const agentMode = ref<AgentMode>("agent");
  const planMode = ref(false);
  const isCreatingNewChat = ref(false);
  const isSidebarOpen = ref(true);
  const toolExecutionLogs = ref<ToolExecutionEntry[]>([]);
  const currentTurnToolIds = ref<string[]>([]);
  const chatScreenRef = ref<ChatScreenHandle | null>(null);
  /** AI 主流程错误的临时展示状态：不进消息数组，只保留最新一条，下次发送时清空。 */
  const chatError = ref<string | null>(null);
  const toolInputById = new Map<string, string>();
  const toolNameById = new Map<string, string>();
  const runtimeStateByConversation = new Map<string, ConversationTurnRuntimeState>();
  const activeRuntimeRefs = {
    isGenerating,
    currentStage,
    assistantResponse,
    assistantReasoning,
    assistantSegments,
    assistantTokenUsage,
    assistantTurnCost,
    pendingQuestion,
    pendingPermissionRequestId,
    currentToolStartedAt,
    currentToolCalls,
    currentToolDurationMs,
    currentContextUsage,
    currentContextCompacts,
    currentContextTokens,
    currentInputTokens,
    currentOutputTokens,
    currentTurnId,
    toolExecutionLogs,
    currentTurnToolIds,
    toolInputById,
    toolNameById,
  };
  const activeRuntimeState = bindActiveRuntimeState(activeRuntimeRefs, () => agentMode.value);
  const currentTurnToolExecutionLogs = computed(() => {
    const ids = new Set(currentTurnToolIds.value);
    return toolExecutionLogs.value.filter((entry) => ids.has(entry.id));
  });
  const latestPersistedPromptTokens = computed(() => {
    for (let index = messages.value.length - 1; index >= 0; index -= 1) {
      const message = messages.value[index];
      if (message.role === "assistant" && (message.cost?.inputTokens ?? 0) > 0) {
        return message.cost?.inputTokens ?? 0;
      }
    }
    return 0;
  });
  const displayContextUsage = computed<ContextUsage | undefined>(() => {
    if ((currentContextUsage.value?.usedTokens ?? 0) > 0) {
      return currentContextUsage.value;
    }
    if (latestPersistedPromptTokens.value > 0) {
      return {
        usedTokens: latestPersistedPromptTokens.value,
        source: "actual",
      };
    }
    return undefined;
  });
  const displayContextTokens = computed(() => {
    if (currentContextTokens.value > 0) {
      return currentContextTokens.value;
    }
    return latestPersistedPromptTokens.value;
  });

  let unlistenChatStream: UnlistenFn | null = null;
  let unlistenScheduledTaskTrigger: UnlistenFn | null = null;

  function persistToolExecutionLog(entry: ToolExecutionEntry, conversationId = activeConversationId.value) {
    if (!conversationId || entry.status === "running") {
      return;
    }

    void upsertConversationToolLog(conversationId, entry).catch((err) => {
      console.error("Failed to persist tool execution log:", err);
    });
  }

  function hasConversationContent(): boolean {
    return messages.value.some(
      (m) => m.content.trim().length > 0 || (m.reasoning?.trim().length ?? 0) > 0 || (m.attachments?.length ?? 0) > 0,
    );
  }

  function handleAgentModeChange(mode: AgentMode) {
    agentMode.value = mode;
    planMode.value = mode === "plan";
  }

  const isCompacting = ref(false);

  async function handleCompactConversation() {
    const conversationId = activeConversationId.value;
    if (!conversationId || isCompacting.value) return;
    isCompacting.value = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const outcome = await invoke<{
        beforeTokens: number;
        afterTokens: number;
        savedTokens: number;
        summary: string;
      }>("manual_compact_conversation", { conversationId });
      await conversationOps.loadConversation(conversationId);
      currentContextCompacts.value = [];
      currentContextTokens.value = outcome.afterTokens;
      currentContextUsage.value = {
        usedTokens: outcome.afterTokens,
        source: "actual",
      };
      emitToast({
        variant: "success",
        source: "manual-compact",
        message: `对话已压缩，节省 ${outcome.savedTokens} tokens`,
      });
    } catch (err) {
      emitToast({
        variant: "error",
        source: "manual-compact",
        message: `压缩失败: ${err}`,
      });
    } finally {
      isCompacting.value = false;
    }
  }

  const conversationOps = createConversationOperations({
    activeConversationId,
    activeWorkspacePath,
    agentMode,
    planMode,
    isGenerating,
    isCreatingNewChat,
    conversations,
    messages,
    toolExecutionLogs,
    conversationFiles,
    pendingUploads,
    conversationMemory,
    assistantResponse,
    assistantReasoning,
    assistantSegments,
    assistantTokenUsage,
    assistantTurnCost,
    runtimeStateByConversation,
    activeRuntimeRefs,
    hasConversationContent,
  });

  const streamOps = createChatStreamOperations({
    activeRuntimeRefs,
    activeRuntimeState,
    activeConversationId,
    agentMode,
    planMode,
    messages,
    runtimeStateByConversation,
    persistMessage: conversationOps.persistMessage,
    persistConversationMemory: conversationOps.persistConversationMemory,
    persistToolExecutionLog,
    cancelActiveConversation: () => cancelChatMessage(activeConversationId.value || null),
    submitPermissionDecision,
  });

  const sendOps = createSendOperations({
    activeConversationId,
    isGenerating,
    currentStage,
    messages,
    toolExecutionLogs,
    pendingUploads,
    pendingPermissionRequestId,
    mainView,
    planMode,
    agentMode,
    assistantResponse,
    assistantReasoning,
    assistantSegments,
    assistantTokenUsage,
    assistantTurnCost,
    currentToolStartedAt,
    currentToolCalls,
    currentToolDurationMs,
    currentContextUsage,
    currentContextCompacts,
    currentContextTokens,
    currentInputTokens,
    currentOutputTokens,
    currentTurnId,
    chatScreenRef,
    runtimeStateByConversation,
    activeRuntimeRefs,
    createNewConversation: conversationOps.createNewConversation,
    persistMessage: conversationOps.persistMessage,
    refreshConversationFiles: conversationOps.refreshConversationFiles,
    resetBackgroundRuntimeState: streamOps.resetBackgroundRuntimeState,
  });

  async function handleNewChat() {
    mainView.value = "chat";
    chatError.value = null;
    resetPendingPromptState(activeRuntimeRefs);
    await conversationOps.handleNewChat();
  }

  async function handleSelectConversation(id: string) {
    mainView.value = "chat";
    chatError.value = null;
    await conversationOps.handleSelectConversation(id);
  }

  async function handleDeleteConversation(id: string) {
    chatError.value = null;
    await conversationOps.handleDeleteConversation(id);
  }

  async function handleSendMessageWithErrorReset(userText: string) {
    chatError.value = null;
    await sendOps.handleSendMessage(userText);
  }

  async function handleEditMessageWithErrorReset(
    payload: { index: number; content: string; id?: string },
  ) {
    chatError.value = null;
    await sendOps.handleEditMessage(payload);
  }

  function dismissChatError() {
    chatError.value = null;
  }

  function onChatErrorEvent(event: Event) {
    const detail = (event as CustomEvent<ChatErrorPayload>).detail;
    const message = detail?.message?.trim();
    if (!message) return;
    // 一个会话同一时刻只保留最新一条错误，新错误直接覆盖旧错误。
    chatError.value = message;
    void chatScreenRef.value?.scrollLiveAssistantIntoView();
  }

  function handleChangeMainView(view: MainView) {
    mainView.value = view;
  }

  function routeChatStreamEvent(payload: ChatMessageEvent) {
    const payloadConversationId = (payload.conversation_id ?? "").trim();
    const targetConversationId = payloadConversationId || activeConversationId.value;
    if (!targetConversationId) {
      return;
    }

    if (targetConversationId !== activeConversationId.value) {
      void streamOps.handleChatStreamEvent(targetConversationId, payload, "background");
      return;
    }
    void streamOps.handleChatStreamEvent(targetConversationId, payload, "active");
  }

  onMounted(async () => {
    // 先确定启动时要恢复的会话，再注册流事件监听。
    // 刷新页面时后端轮次仍在运行：若在 loadConversation 完成前处理该会话的
    // 流事件，事件会因 activeConversationId 为空而被当成"后台会话"，用空白
    // 状态只累积到尾部增量，finalizeBackgroundTurn 会把残缺尾部落盘并 ack 掉
    // 后端完整快照，恢复机制随之失效（历史中出现被截断的消息）。
    await conversationOps.refreshConversations();
    const startupConversationId = conversations.value[0]?.id ?? "";
    // 恢复完成前丢弃目标会话的流事件：后端 live_turns 快照包含全部增量，
    // 恢复以快照为准，这些事件是冗余的，处理反而会产生残缺/重复内容。
    let startupRestorePending = startupConversationId.length > 0;

    try {
      unlistenChatStream = await listen<ChatMessageEvent>("chat-stream", (event) => {
        const payload = event.payload;
        if (
          startupRestorePending &&
          (payload.conversation_id ?? "").trim() === startupConversationId
        ) {
          return;
        }
        routeChatStreamEvent(payload);
      });
    } catch (err) {
      console.error("Failed to setup listener:", err);
    }

    if (startupConversationId) {
      await conversationOps.loadConversation(startupConversationId);
      startupRestorePending = false;
      // loadConversation 期间后端可能又推进了若干增量，以最新快照再恢复一次，
      // 避免正文在恢复窗口内出现缺口；若轮次已在窗口内结束，则落盘完整内容。
      await conversationOps.restoreActiveLiveTurn();
    }

    try {
      unlistenScheduledTaskTrigger = await listen<ScheduledTaskTriggerEvent>(
        "scheduled-task-trigger",
        (event) => {
          const payload = event.payload;
          const promptPreview = (payload.prompt ?? "").trim();
          const previewText =
            promptPreview.length > 70
              ? `${promptPreview.slice(0, 70)}...`
              : promptPreview;

          emitToast({
            variant: "info",
            source: "schedule",
            message: `定时任务触发: ${payload.id} (${payload.cron})${payload.conversationId ? ` [${payload.conversationId}]` : ""}${previewText ? ` - ${previewText}` : ""}`,
          });
        },
      );
    } catch (err) {
      console.error("Failed to setup scheduled-task-trigger listener:", err);
    }

    window.addEventListener("history-cleared", conversationOps.handleHistoryCleared as EventListener);
    window.addEventListener(NOVA_CHAT_ERROR_EVENT, onChatErrorEvent as EventListener);
  });

  onUnmounted(() => {
    if (unlistenChatStream) unlistenChatStream();
    if (unlistenScheduledTaskTrigger) unlistenScheduledTaskTrigger();
    window.removeEventListener("history-cleared", conversationOps.handleHistoryCleared as EventListener);
    window.removeEventListener(NOVA_CHAT_ERROR_EVENT, onChatErrorEvent as EventListener);
  });

  return {
    messages,
    isGenerating,
    currentStage,
    assistantResponse,
    assistantReasoning,
    assistantSegments,
    assistantTokenUsage,
    assistantTurnCost,
    toolExecutionLogs,
    conversations,
    activeConversationId,
    activeWorkspacePath,
    pendingQuestion,
    pendingPermissionRequestId,
    pendingUploads,
    conversationFiles,
    currentContextUsage: displayContextUsage,
    currentContextCompacts,
    currentContextTokens: displayContextTokens,
    agentMode,
    planMode,
    currentTurnToolExecutionLogs,
    mainView,
    isSidebarOpen,
    chatScreenRef,
    chatError,
    dismissChatError,
    refreshActiveConversationFiles: conversationOps.refreshActiveConversationFiles,
    handleSendMessage: handleSendMessageWithErrorReset,
    handleEditMessage: handleEditMessageWithErrorReset,
    handleUploadFiles: sendOps.handleUploadFiles,
    handleRemovePendingUpload: sendOps.handleRemovePendingUpload,
    handleCancelGeneration: sendOps.handleCancelGeneration,
    handlePendingQuestionSubmit: sendOps.handlePendingQuestionSubmit,
    handlePendingQuestionSkip: sendOps.handlePendingQuestionSkip,
    handleAgentModeChange,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation,
    handlePinConversation: conversationOps.handlePinConversation,
    handleChangeMainView,
    isCompacting,
    handleCompactConversation,
  };
}
