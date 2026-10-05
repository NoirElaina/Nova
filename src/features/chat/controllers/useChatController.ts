import { computed, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import { storeToRefs } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { emitToast, NOVA_CHAT_ERROR_EVENT, type ChatErrorPayload } from "../../../lib/toast";
import {
  getConversationUsage,
} from "../services/chat-api";
import type {
  AgentMode,
  AssistantTranscriptSegment,
  ChatMessage,
  ContextCompactSummary,
  ConversationUsageSummary,
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
  resetPendingPromptState,
} from "./chat-runtime-state";
import { createConversationOperations } from "./chat-conversation-ops";
import { createSendOperations } from "./chat-send-ops";
import { setupAgentEventListener } from "../../agent/agent-listener";
import { useConversationStore } from "@/stores/conversation";
import { useAgentSessionStore } from "@/stores/agentSession";

export function useChatController() {
  const conversationStore = useConversationStore();
  const agentSessionStore = useAgentSessionStore();

  const {
    conversations,
    activeConversationId,
    activeWorkspacePath,
    conversationFiles,
    isSidebarOpen,
    mainView,
    pendingAgentBundleId,
  } = storeToRefs(conversationStore);

  const messages = shallowRef<ChatMessage[]>([]);
  const isGenerating = ref(false);
  const currentStage = ref<LiveTurnStage>("processing");
  const assistantResponse = ref("");
  const assistantReasoning = ref("");
  const assistantSegments = ref<AssistantTranscriptSegment[]>([]);
  const assistantTokenUsage = ref<number | undefined>(undefined);
  const assistantTurnCost = ref<TurnCost | undefined>(undefined);
  const pendingUploads = ref<PendingUploadFile[]>([]);
  const pendingQuestion = ref<NeedsUserInputPayload | null>(null);
  const pendingPermissionRequestId = ref<string | null>(null);
  const conversationUsage = ref<ConversationUsageSummary | null>(null);
  const currentToolStartedAt = ref<number | null>(null);
  const currentToolCalls = ref(0);
  const currentToolDurationMs = ref(0);
  const currentContextUsage = ref<ContextUsage | undefined>(undefined);
  const currentContextCompacts = ref<ContextCompactSummary[]>([]);
  const currentContextTokens = ref(0);
  const currentInputTokens = ref(0);
  const currentOutputTokens = ref(0);
  const currentTurnId = ref<string | null>(null);
  const currentTurnStartedAt = ref<number | null>(null);
  const agentMode = ref<AgentMode>("agent");
  const isCreatingNewChat = ref(false);
  const toolExecutionLogs = ref<ToolExecutionEntry[]>([]);

  // 保证 Pinia 会话运行态 Store 与 Controller 状态双向连通，彻底废除各自孤立更新
  watch(
    messages,
    (val) => {
      agentSessionStore.activeSession.messages = val;
    },
    { immediate: true, deep: false },
  );
  watch(
    toolExecutionLogs,
    (val) => {
      agentSessionStore.activeSession.toolExecutionLogs = val;
    },
    { immediate: true, deep: false },
  );
  watch(
    isGenerating,
    (val) => {
      agentSessionStore.activeSession.isGenerating = val;
    },
    { immediate: true },
  );
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
    currentTurnStartedAt,
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
    isGenerating,
    isCreatingNewChat,
    conversations,
    messages,
    toolExecutionLogs,
    conversationFiles,
    pendingUploads,
    conversationUsage,
    assistantResponse,
    assistantReasoning,
    assistantSegments,
    assistantTokenUsage,
    assistantTurnCost,
    runtimeStateByConversation,
    activeRuntimeRefs,
    hasConversationContent,
  });

  function resetBackgroundRuntimeState(
    _conversationId: string,
    state: ConversationTurnRuntimeState,
    _preservePendingPrompt?: boolean,
  ) {
    state.isGenerating = false;
    state.currentStage = "processing";
    state.assistantResponse = "";
    state.assistantReasoning = "";
    state.assistantSegments = [];
    state.toolExecutionLogs = [];
  }

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
    isGenerating,
    currentStage,
    messages,
    toolExecutionLogs,
    pendingUploads,
    pendingPermissionRequestId,
    mainView,
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
    currentTurnStartedAt,
    pendingAgentBundleId,
    chatScreenRef,
    runtimeStateByConversation,
    activeRuntimeRefs,
    createNewConversation: conversationOps.createNewConversation,
    persistMessage: conversationOps.persistMessage,
    refreshConversationFiles: conversationOps.refreshConversationFiles,
    resetBackgroundRuntimeState,
    finalizeActiveTurnOnError,
  });

  async function handleNewChat() {
    mainView.value = "chat";
    chatError.value = null;
    pendingAgentBundleId.value = null;
    resetPendingPromptState(activeRuntimeRefs);
    await conversationOps.handleNewChat();
  }

  /** 智能体页点「启用」：不建会话，只暂存智能体并回到欢迎页；首次发送时才创建对话并挂载。 */
  async function handleLaunchAgentConversation(bundleId: string) {
    mainView.value = "chat";
    chatError.value = null;
    pendingAgentBundleId.value = bundleId;
    resetPendingPromptState(activeRuntimeRefs);
    await conversationOps.handleNewChat();
  }

  function clearPendingAgent() {
    pendingAgentBundleId.value = null;
  }

  async function handleSelectConversation(id: string) {
    mainView.value = "chat";
    chatError.value = null;
    pendingAgentBundleId.value = null;
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
          isGenerating.value = true;
          currentStage.value = "processing";
          assistantResponse.value = "";
          assistantReasoning.value = "";
          assistantSegments.value = [];
        },
        onStateChanged: (_turnId, state) => {
          agentSessionStore.handleStateChanged(state);
        },
        onThinkingDelta: (delta) => {
          agentSessionStore.handleThinkingDelta(delta);
          assistantReasoning.value += delta;
        },
        onTextDelta: (delta) => {
          agentSessionStore.handleTextDelta(delta);
          assistantResponse.value += delta;
        },
        onToolRequested: (callId, toolName, args) => {
          agentSessionStore.handleToolRequested(callId, toolName, args);
          toolExecutionLogs.value.push({
            id: callId,
            toolName,
            status: "running",
            input: typeof args === "string" ? args : JSON.stringify(args, null, 2),
            result: "",
            startedAt: Date.now(),
          });
        },
        onToolCompleted: (callId, _toolName, output, isError, _durationMs) => {
          agentSessionStore.handleToolCompleted(callId, _toolName, output, isError, _durationMs);
          const entry = toolExecutionLogs.value.find((e) => e.id === callId);
          if (entry) {
            entry.status = isError ? "error" : "completed";
            entry.result = output;
            entry.finishedAt = Date.now();
          }
        },
        onVerificationStarted: (target) => {
          emitToast({
            variant: "info",
            source: "verifier",
            message: `[即时自愈] 正在对 ${target} 进行静态语法树分析...`,
          });
        },
        onVerificationCompleted: (target, passed, feedback) => {
          if (!passed) {
            emitToast({
              variant: "warning",
              source: "verifier",
              message: `[自愈拦截] ${target} 发生语法错误，已阻断写入并回灌模型: ${feedback || ""}`,
            });
          }
        },
        onTokenUsage: (usage) => {
          agentSessionStore.handleTokenUsage(usage);
          assistantTokenUsage.value = usage.input + usage.output;
        },
        onTurnFinished: async (_turnId, _stopReason) => {
          await agentSessionStore.handleTurnFinished(_turnId, _stopReason);
          isGenerating.value = false;
          if (activeConversationId.value) {
            await conversationOps.loadConversation(activeConversationId.value);
            void getConversationUsage(activeConversationId.value)
              .then((usage) => {
                if (activeConversationId.value) {
                  conversationUsage.value = usage;
                }
              })
              .catch(() => {});
          }
        },
        onError: (error) => {
          agentSessionStore.handleTurnError(error);
          isGenerating.value = false;
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

    window.addEventListener("history-cleared", conversationOps.handleHistoryCleared as EventListener);
    window.addEventListener(NOVA_CHAT_ERROR_EVENT, onChatErrorEvent as EventListener);
  });

  onUnmounted(() => {
    if (unlistenScheduledTaskTrigger) unlistenScheduledTaskTrigger();
    if (unlistenAgentEvent) unlistenAgentEvent();
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
