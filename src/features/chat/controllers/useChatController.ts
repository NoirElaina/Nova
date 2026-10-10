import { ref, computed, onMounted, onUnmounted } from "vue";
import { storeToRefs } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { emitToast, NOVA_CHAT_ERROR_EVENT, type ChatErrorPayload } from "../../../lib/toast";
import type {
  AgentMode,
  ContextUsage,
} from "../../../lib/chat-types";
import {
  getConversationUsage,
  manualCompactConversation,
} from "../services/chat-api";
import {
  type ChatScreenHandle,
  type MainView,
  type ScheduledTaskTriggerEvent,
} from "./chat-controller-types";
import { createConversationOperations } from "./chat-conversation-ops";
import { createSendOperations } from "./chat-send-ops";
import { setupAgentEventListener } from "../../agent/agent-listener";
import { useConversationStore } from "@/stores/conversation";
import { useAgentSessionStore } from "@/stores/agentSession";
import { useComposerStore } from "@/stores/composer";

function createChatController() {
  const conversationStore = useConversationStore();
  const agentSessionStore = useAgentSessionStore();
  const composerStore = useComposerStore();

  const {
    activeConversationId,
    activeWorkspacePath,
    conversations,
    conversationFiles,
    pendingAgentBundleId,
    mainView,
    isSidebarOpen,
  } = storeToRefs(conversationStore);

  const { pendingUploads } = storeToRefs(composerStore);

  const activeSession = computed(() => agentSessionStore.activeSession);
  const messages = computed(() => activeSession.value.messages);
  const isGenerating = computed(() => activeSession.value.isGenerating);
  const currentStage = computed(() => activeSession.value.currentStage);
  const assistantResponse = computed(() => activeSession.value.assistantResponse);
  const assistantReasoning = computed(() => activeSession.value.assistantReasoning);
  const assistantSegments = computed(() => activeSession.value.assistantSegments);
  const assistantTokenUsage = computed(() => activeSession.value.assistantTokenUsage);
  const assistantTurnCost = computed(() => activeSession.value.assistantTurnCost);
  const toolExecutionLogs = computed(() => activeSession.value.toolExecutionLogs);
  const pendingQuestion = computed(() => activeSession.value.pendingQuestion);
  const pendingPermissionRequestId = computed(() => activeSession.value.pendingPermissionRequestId);
  const conversationUsage = computed(() => activeSession.value.conversationUsage);
  const currentTurnStartedAt = computed(() => activeSession.value.currentTurnStartedAt);
  const currentContextUsage = computed(() => activeSession.value.contextUsage);
  const currentContextCompacts = computed(() => activeSession.value.contextCompacts);
  const currentContextTokens = computed(() => activeSession.value.contextTokens);
  const chatError = computed(() => activeSession.value.chatError);

  const agentMode = ref<AgentMode>("agent");
  const chatScreenRef = ref<ChatScreenHandle | null>(null);

  const currentTurnToolExecutionLogs = computed(() => {
    return toolExecutionLogs.value;
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

  let unlistenScheduledTaskTrigger: UnlistenFn | null = null;
  let unlistenAgentEvent: UnlistenFn | null = null;

  function hasConversationContent(): boolean {
    return messages.value.some(
      (m) => m.content.trim().length > 0 || (m.reasoning?.trim().length ?? 0) > 0 || (m.attachments?.length ?? 0) > 0,
    );
  }

  const isCompacting = ref(false);

  async function handleCompactConversation() {
    const conversationId = activeConversationId.value;
    if (!conversationId || isCompacting.value) return;
    isCompacting.value = true;
    try {
      const outcome = await manualCompactConversation(conversationId);
      await conversationOps.loadConversation(conversationId);
      activeSession.value.contextTokens = outcome.afterTokens;
      emitToast({
        variant: "success",
        source: "compact",
        message: `上下文已压缩：节省约 ${outcome.savedTokens} tokens (${outcome.beforeTokens} -> ${outcome.afterTokens})`,
      });
    } catch (err: unknown) {
      const rawMsg = err instanceof Error ? err.message : String(err);
      emitToast({
        variant: "error",
        source: "compact",
        message: `压缩失败: ${rawMsg}`,
      });
    } finally {
      isCompacting.value = false;
    }
  }

  const conversationOps = createConversationOperations({
    activeConversationId,
    activeWorkspacePath,
    agentMode,
    conversations,
    conversationFiles,
    hasConversationContent,
  });

  async function finalizeActiveTurnOnError() {
    const content = assistantResponse.value.trim();
    if (content && activeConversationId.value) {
      await conversationOps.persistMessage(
        {
          id: `msg-${Date.now()}`,
          role: "assistant",
          content: `${content}\n\n（本轮因错误中断）`,
          reasoning: assistantReasoning.value.trim() || undefined,
          createdAt: Date.now(),
        },
        activeConversationId.value,
      );
    }
  }

  const sendOps = createSendOperations({
    activeConversationId,
    mainView,
    agentMode,
    pendingAgentBundleId,
    chatScreenRef,
    createNewConversation: conversationOps.createNewConversation,
    refreshConversationFiles: conversationOps.refreshConversationFiles,
    finalizeActiveTurnOnError,
  });


  async function handleNewChat() {
    mainView.value = "chat";
    agentSessionStore.dismissChatError();
    pendingAgentBundleId.value = null;
    await conversationOps.handleNewChat();
  }

  /** 智能体页点「启用」：不建会话，只暂存智能体并回到欢迎页；首次发送时才创建对话并挂载。 */
  async function handleLaunchAgentConversation(bundleId: string) {
    mainView.value = "chat";
    agentSessionStore.dismissChatError();
    pendingAgentBundleId.value = bundleId;
    await conversationOps.handleNewChat();
  }

  function clearPendingAgent() {
    pendingAgentBundleId.value = null;
  }

  async function handleSelectConversation(id: string) {
    mainView.value = "chat";
    agentSessionStore.dismissChatError();
    pendingAgentBundleId.value = null;
    await conversationOps.handleSelectConversation(id);
  }

  async function handleDeleteConversation(id: string) {
    agentSessionStore.dismissChatError();
    await conversationOps.handleDeleteConversation(id);
  }

  async function handleSendMessageWithErrorReset(userText: string) {
    agentSessionStore.dismissChatError();
    await sendOps.handleSendMessage(userText);
  }

  async function handleEditMessageWithErrorReset(
    payload: { index: number; content: string; id?: string },
  ) {
    agentSessionStore.dismissChatError();
    await sendOps.handleEditMessage(payload);
  }

  function dismissChatError() {
    agentSessionStore.dismissChatError();
  }

  function onChatErrorEvent(event: Event) {
    const detail = (event as CustomEvent<ChatErrorPayload>).detail;
    const message = detail?.message?.trim();
    if (!message) return;
    agentSessionStore.activeSession.chatError = message;
    void chatScreenRef.value?.scrollLiveAssistantIntoView();
  }

  function handleChangeMainView(view: MainView) {
    mainView.value = view;
  }

  onMounted(async () => {
    await conversationOps.refreshConversations();
    const startupConversationId = conversations.value[0]?.id ?? "";
    if (startupConversationId) {
      await conversationOps.loadConversation(startupConversationId);
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

    try {
      unlistenAgentEvent = await setupAgentEventListener({
        onTurnStarted: (_turnId, _convId) => {
          agentSessionStore.handleTurnStarted(_turnId, _convId);
        },
        onStateChanged: (_turnId, state) => {
          agentSessionStore.handleStateChanged(state, _turnId);
        },
        onThinkingDelta: (delta, turnId) => {
          agentSessionStore.handleThinkingDelta(delta, turnId);
        },
        onTextDelta: (delta, turnId) => {
          agentSessionStore.handleTextDelta(delta, turnId);
        },
        onToolRequested: (callId, toolName, args, turnId) => {
          agentSessionStore.handleToolRequested(callId, toolName, args, turnId);
        },
        onToolCompleted: (callId, _toolName, output, isError, _durationMs, turnId) => {
          agentSessionStore.handleToolCompleted(callId, _toolName, output, isError, _durationMs, turnId);
        },
        onTokenUsage: (usage, turnId) => {
          agentSessionStore.handleTokenUsage(usage, turnId);
        },
        onPermissionRequested: (turnId, requestId, toolName, payload) => {
          agentSessionStore.handlePermissionRequested(requestId, toolName, payload, turnId);
        },
        onTurnFinished: async (_turnId, _stopReason) => {
          await agentSessionStore.handleTurnFinished(_turnId, _stopReason, _turnId);
          if (activeConversationId.value) {
            void getConversationUsage(activeConversationId.value)
              .then((usage) => {
                if (activeConversationId.value) {
                  agentSessionStore.activeSession.conversationUsage = usage;
                }
              })
              .catch(() => {});
          }
        },
        onError: (error, turnId) => {
          agentSessionStore.handleTurnError(error, turnId);
          emitToast({
            variant: "error",
            source: "agent",
            message: error,
          });
        },
      });
    } catch (err) {
      console.error("Failed to setup modern agent event listener:", err);
    }

    window.addEventListener("history-cleared", conversationOps.handleHistoryCleared as unknown as EventListener);
    window.addEventListener(NOVA_CHAT_ERROR_EVENT, onChatErrorEvent as EventListener);
  });

  onUnmounted(() => {
    if (unlistenScheduledTaskTrigger) unlistenScheduledTaskTrigger();
    if (unlistenAgentEvent) unlistenAgentEvent();
    window.removeEventListener("history-cleared", conversationOps.handleHistoryCleared as unknown as EventListener);
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
    conversationUsage,
    currentTurnStartedAt,
    currentTurnToolExecutionLogs,
    mainView,
    isSidebarOpen,
    chatScreenRef,
    chatError,
    dismissChatError,
    refreshActiveConversationFiles: conversationOps.refreshActiveConversationFiles,
    handleSendMessage: handleSendMessageWithErrorReset,
    handleEditMessage: handleEditMessageWithErrorReset,
    handleLaunchAgentConversation,
    clearPendingAgent,
    pendingAgentBundleId,
    handleUploadFiles: sendOps.handleUploadFiles,
    handleRemovePendingUpload: sendOps.handleRemovePendingUpload,
    handleCancelGeneration: sendOps.handleCancelGeneration,
    handlePendingQuestionSubmit: sendOps.handlePendingQuestionSubmit,
    handlePendingQuestionSkip: sendOps.handlePendingQuestionSkip,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation,
    handlePinConversation: conversationOps.handlePinConversation,
    handleChangeMainView,
    isCompacting,
    handleCompactConversation,
  };
}

export type ChatController = ReturnType<typeof createChatController>;

let singletonInstance: ChatController | null = null;

export function useChatController(): ChatController {
  if (!singletonInstance) {
    singletonInstance = createChatController();
  }
  return singletonInstance;
}

export function resetChatControllerForTesting(): void {
  singletonInstance = null;
}

