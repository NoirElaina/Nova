import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { emitToast } from "../../../lib/toast";
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
  const messages = ref<ChatMessage[]>([]);
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
  const activeRuntimeState = bindActiveRuntimeState(activeRuntimeRefs);
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
    resetPendingPromptState(activeRuntimeRefs);
    await conversationOps.handleNewChat();
  }

  async function handleSelectConversation(id: string) {
    mainView.value = "chat";
    await conversationOps.handleSelectConversation(id);
  }

  function handleChangeMainView(view: MainView) {
    mainView.value = view;
  }

  onMounted(async () => {
    try {
      unlistenChatStream = await listen<ChatMessageEvent>("chat-stream", (event) => {
        const payload = event.payload;
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
      });
    } catch (err) {
      console.error("Failed to setup listener:", err);
    }

    await conversationOps.refreshConversations();
    if (conversations.value.length > 0) {
      await conversationOps.loadConversation(conversations.value[0].id);
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
  });

  onUnmounted(() => {
    if (unlistenChatStream) unlistenChatStream();
    if (unlistenScheduledTaskTrigger) unlistenScheduledTaskTrigger();
    window.removeEventListener("history-cleared", conversationOps.handleHistoryCleared as EventListener);
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
    refreshActiveConversationFiles: conversationOps.refreshActiveConversationFiles,
    handleSendMessage: sendOps.handleSendMessage,
    handleEditMessage: sendOps.handleEditMessage,
    handleUploadFiles: sendOps.handleUploadFiles,
    handleRemovePendingUpload: sendOps.handleRemovePendingUpload,
    handleCancelGeneration: sendOps.handleCancelGeneration,
    handlePendingQuestionSubmit: sendOps.handlePendingQuestionSubmit,
    handlePendingQuestionSkip: sendOps.handlePendingQuestionSkip,
    handleAgentModeChange,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation: conversationOps.handleDeleteConversation,
    handlePinConversation: conversationOps.handlePinConversation,
    handleChangeMainView,
    isCompacting,
    handleCompactConversation,
  };
}
