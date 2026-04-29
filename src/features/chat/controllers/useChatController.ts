import { ref, onMounted, onUnmounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { emitToast } from "../../../lib/toast";
import {
  cancelChatMessage,
  submitPermissionDecision,
  type RagDocumentMeta,
  upsertConversationToolLog,
} from "../services/chat-api";
import type {
  AgentMode,
  ChatMessage,
  ChatMessageEvent,
  ConversationMemory,
  ConversationMeta,
  NeedsUserInputPayload,
  PendingUploadFile,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";
import {
  type BackendErrorEvent,
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
  const assistantResponse = ref("");
  const assistantReasoning = ref("");
  const assistantTokenUsage = ref<number | undefined>(undefined);
  const assistantTurnCost = ref<TurnCost | undefined>(undefined);
  const conversations = ref<ConversationMeta[]>([]);
  const activeConversationId = ref("");
  const conversationFiles = ref<RagDocumentMeta[]>([]);
  const pendingUploads = ref<PendingUploadFile[]>([]);
  const pendingQuestion = ref<NeedsUserInputPayload | null>(null);
  const pendingPermissionRequestId = ref<string | null>(null);
  const conversationMemory = ref<ConversationMemory | null>(null);
  const mainView = ref<MainView>("chat");
  const currentToolStartedAt = ref<number | null>(null);
  const currentToolCalls = ref(0);
  const currentToolDurationMs = ref(0);
  const currentInputTokens = ref(0);
  const currentOutputTokens = ref(0);
  const agentMode = ref<AgentMode>("agent");
  const planMode = ref(false);
  const isCreatingNewChat = ref(false);
  const isSidebarOpen = ref(true);
  const toolExecutionLogs = ref<ToolExecutionEntry[]>([]);
  const chatScreenRef = ref<ChatScreenHandle | null>(null);
  const toolInputById = new Map<string, string>();
  const toolNameById = new Map<string, string>();
  const runtimeStateByConversation = new Map<string, ConversationTurnRuntimeState>();
  const activeRuntimeRefs = {
    isGenerating,
    assistantResponse,
    assistantReasoning,
    assistantTokenUsage,
    assistantTurnCost,
    pendingQuestion,
    pendingPermissionRequestId,
    currentToolStartedAt,
    currentToolCalls,
    currentToolDurationMs,
    currentInputTokens,
    currentOutputTokens,
    toolExecutionLogs,
    toolInputById,
    toolNameById,
  };
  const activeRuntimeState = bindActiveRuntimeState(activeRuntimeRefs);

  let unlistenChatStream: UnlistenFn | null = null;
  let unlistenBackendError: UnlistenFn | null = null;
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

  const conversationOps = createConversationOperations({
    activeConversationId,
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
    chatScreenRef,
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
    messages,
    pendingUploads,
    pendingPermissionRequestId,
    conversationMemory,
    mainView,
    planMode,
    agentMode,
    assistantResponse,
    assistantReasoning,
    assistantTokenUsage,
    assistantTurnCost,
    currentToolStartedAt,
    currentToolCalls,
    currentToolDurationMs,
    currentInputTokens,
    currentOutputTokens,
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
    await conversationOps.refreshConversations();
    if (conversations.value.length === 0) {
      const id = await conversationOps.createNewConversation("New chat");
      if (id) {
        await conversationOps.loadConversation(id);
      }
    } else {
      await conversationOps.loadConversation(conversations.value[0].id);
    }

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

    try {
      unlistenBackendError = await listen<BackendErrorEvent>("backend-error", (event) => {
        const payload = event.payload ?? {};
        const prefix = [payload.source, payload.stage].filter(Boolean).join(" / ");
        const message = payload.message || "后端工作流发生未知错误";
        emitToast({
          variant: "error",
          source: "backend-error",
          message: prefix ? `[${prefix}] ${message}` : message,
        });
      });
    } catch (err) {
      console.error("Failed to setup backend-error listener:", err);
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
    if (unlistenBackendError) unlistenBackendError();
    if (unlistenScheduledTaskTrigger) unlistenScheduledTaskTrigger();
    window.removeEventListener("history-cleared", conversationOps.handleHistoryCleared as EventListener);
  });

  return {
    messages,
    isGenerating,
    assistantResponse,
    assistantReasoning,
    assistantTokenUsage,
    assistantTurnCost,
    toolExecutionLogs,
    conversations,
    activeConversationId,
    pendingQuestion,
    pendingUploads,
    conversationFiles,
    agentMode,
    planMode,
    mainView,
    isSidebarOpen,
    chatScreenRef,
    refreshActiveConversationFiles: conversationOps.refreshActiveConversationFiles,
    handleSendMessage: sendOps.handleSendMessage,
    handleUploadFiles: sendOps.handleUploadFiles,
    handleRemovePendingUpload: sendOps.handleRemovePendingUpload,
    handleCancelGeneration: sendOps.handleCancelGeneration,
    handlePendingQuestionSubmit: sendOps.handlePendingQuestionSubmit,
    handlePendingQuestionSkip: sendOps.handlePendingQuestionSkip,
    handleAgentModeChange,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation: conversationOps.handleDeleteConversation,
    handleChangeMainView,
  };
}
