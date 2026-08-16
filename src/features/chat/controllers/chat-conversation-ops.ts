import type { Ref } from "vue";
import { emitToast } from "../../../lib/toast";
import type {
  AgentMode,
  AssistantTranscriptSegment,
  ChatMessage,
  ConversationMemory,
  ConversationMeta,
  PendingUploadFile,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";
import {
  ackChatTurnStatus,
  appendConversationMessage,
  createConversation,
  deleteConversation,
  getChatTurnStatus,
  getConversationMemory,
  listSessionFiles,
  listConversations,
  loadConversationHistory,
  loadConversationToolLogs,
  setConversationPinned,
  type SessionFileMeta,
  upsertConversationMemory,
} from "../services/chat-api";
import { extractSessionMemory } from "../utils/session-memory";
import { clearBrowserTabState } from "../../browser/browser-tab-state";
import type { ConversationTurnRuntimeState } from "./chat-controller-types";
import type { ActiveRuntimeRefs } from "./chat-runtime-state";
import {
  clearActiveRuntimeState,
  hasAnyGeneratingConversations,
  isSpecificConversationGenerating,
  normalizeConversationId,
  restoreRuntimeState,
  stashRuntimeState,
} from "./chat-runtime-state";
import { buildAssistantTranscriptSegments } from "../utils/assistant-transcript";

type ConversationOpsDeps = {
  activeConversationId: Ref<string>;
  /** 当前工作区路径（前端状态）。无活跃会话时由 EnvironmentBar 修改；有活跃会话时反映该会话的工作区。 */
  activeWorkspacePath: Ref<string>;
  agentMode: Ref<AgentMode>;
  planMode: Ref<boolean>;
  isGenerating: Ref<boolean>;
  isCreatingNewChat: Ref<boolean>;
  conversations: Ref<ConversationMeta[]>;
  messages: Ref<ChatMessage[]>;
  toolExecutionLogs: Ref<ToolExecutionEntry[]>;
  conversationFiles: Ref<SessionFileMeta[]>;
  pendingUploads: Ref<PendingUploadFile[]>;
  conversationMemory: Ref<ConversationMemory | null>;
  assistantResponse: Ref<string>;
  assistantReasoning: Ref<string>;
  assistantSegments: Ref<AssistantTranscriptSegment[]>;
  assistantTokenUsage: Ref<number | undefined>;
  assistantTurnCost: Ref<TurnCost | undefined>;
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>;
  activeRuntimeRefs: ActiveRuntimeRefs;
  hasConversationContent: () => boolean;
};

export function createConversationOperations(deps: ConversationOpsDeps) {
  const {
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
  } = deps;

  async function refreshConversationFiles(conversationId: string) {
    if (!conversationId) {
      conversationFiles.value = [];
      return;
    }

    try {
      conversationFiles.value = await listSessionFiles(conversationId);
    } catch (err) {
      console.error("Failed to load conversation files:", err);
      conversationFiles.value = [];
    }
  }

  async function refreshActiveConversationFiles() {
    await refreshConversationFiles(activeConversationId.value);
  }

  async function loadConversationMemory(conversationId: string) {
    try {
      const mem = await getConversationMemory(conversationId);
      conversationMemory.value = mem;
    } catch (err) {
      console.error("Failed to load conversation memory:", err);
      conversationMemory.value = null;
    }
  }

  function clearLiveTurnRuntime() {
    isGenerating.value = false;
    activeRuntimeRefs.currentStage.value = "processing";
    assistantResponse.value = "";
    assistantReasoning.value = "";
    assistantSegments.value = [];
    assistantTokenUsage.value = undefined;
    assistantTurnCost.value = undefined;
    activeRuntimeRefs.pendingQuestion.value = null;
    activeRuntimeRefs.pendingPermissionRequestId.value = null;
    activeRuntimeRefs.currentToolStartedAt.value = null;
    activeRuntimeRefs.currentToolCalls.value = 0;
    activeRuntimeRefs.currentToolDurationMs.value = 0;
    activeRuntimeRefs.currentContextUsage.value = undefined;
    activeRuntimeRefs.currentContextCompacts.value = [];
    activeRuntimeRefs.currentContextTokens.value = 0;
    activeRuntimeRefs.currentInputTokens.value = 0;
    activeRuntimeRefs.currentOutputTokens.value = 0;
    activeRuntimeRefs.currentTurnId.value = null;
    activeRuntimeRefs.currentTurnToolIds.value = [];
    activeRuntimeRefs.toolInputById.clear();
    activeRuntimeRefs.toolNameById.clear();
  }

  function isDuplicateAssistantMessage(content: string, reasoning: string) {
    const last = messages.value[messages.value.length - 1];
    return (
      last?.role === "assistant" &&
      last.content.trim() === content.trim() &&
      (last.reasoning ?? "").trim() === reasoning.trim()
    );
  }

  async function restoreLiveTurnStatus(conversationId: string) {
    const liveTurn = await getChatTurnStatus(conversationId);
    if (!liveTurn) {
      return;
    }

    const response = liveTurn.assistantResponse ?? "";
    const reasoning = liveTurn.assistantReasoning ?? "";
    if (liveTurn.state === "running") {
      isGenerating.value = true;
      activeRuntimeRefs.currentStage.value = "processing";
      assistantResponse.value = response;
      assistantReasoning.value = reasoning;
      assistantSegments.value = buildAssistantTranscriptSegments(undefined, {
        reasoning,
        text: response,
      });
      assistantTokenUsage.value = undefined;
      assistantTurnCost.value = undefined;
      return;
    }

    const finalText = response.trim();
    const finalReasoning = reasoning.trim();
    if (finalText || finalReasoning) {
      const content =
        liveTurn.state === "cancelled"
          ? finalText
            ? `${finalText}\n\n（已取消当前轮）`
            : "已取消当前轮。"
          : finalText || "（本轮没有返回可显示的文本内容）";

      if (!isDuplicateAssistantMessage(content, finalReasoning)) {
        const assistantMessage: ChatMessage = {
          id: `assistant-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
          role: "assistant",
          content,
          reasoning: finalReasoning || undefined,
          transcriptSegments: buildAssistantTranscriptSegments(undefined, {
            reasoning: finalReasoning,
            text: finalText,
          }),
          createdAt: Date.now(),
        };
        messages.value = [...messages.value, assistantMessage];
        await persistMessage(assistantMessage, conversationId);
      }
    }

    clearLiveTurnRuntime();
    await ackChatTurnStatus(conversationId);
  }

  /**
   * 基于后端 live_turns 快照对当前会话再执行一次轮次恢复。
   * 快照在页面刷新期间也由后端持续累积，始终是完整数据；
   * 用于启动加载完成后覆盖加载窗口内错过的流式增量。
   */
  async function restoreActiveLiveTurn() {
    const conversationId = activeConversationId.value;
    if (!conversationId) {
      return;
    }
    await restoreLiveTurnStatus(conversationId);
  }

  async function persistConversationMemory(conversationId: string) {
    const { summary, keyFacts } = extractSessionMemory(messages.value);
    if (!summary.trim()) return;
    try {
      await upsertConversationMemory(conversationId, summary, keyFacts);
      conversationMemory.value = {
        summary,
        keyFacts,
        updatedAt: Date.now(),
      };
    } catch (err) {
      console.error("Failed to persist conversation memory:", err);
    }
  }

  async function refreshConversations() {
    try {
      const items = await listConversations();
      conversations.value = (items || []).filter(
        (item) => !item.title.startsWith("Scheduled ["),
      );
    } catch (err) {
      console.error("Failed to list conversations:", err);
    }
  }

  async function createNewConversation(seedTitle?: string): Promise<string | null> {
    try {
      const conv = await createConversation(seedTitle, activeWorkspacePath.value || undefined);
      activeWorkspacePath.value = conv.workspacePath || '';
      await refreshConversations();
      return conv.id;
    } catch (err) {
      console.error("Failed to create conversation:", err);
      return null;
    }
  }

  // 会话加载序号：快速切换会话时多个 loadConversation 会并发飞行，
  // 后发先至的旧结果会覆盖新会话的 UI（消息/记忆/文件串台），
  // 每个 await 之后都校验序号，过期的加载直接放弃写入。
  let conversationLoadSequence = 0;

  async function loadConversation(id: string) {
    const targetConversationId = id.trim();
    if (!targetConversationId) {
      return;
    }

    const loadToken = ++conversationLoadSequence;
    const isStaleLoad = () =>
      loadToken !== conversationLoadSequence ||
      activeConversationId.value !== targetConversationId;

    const previousConversationId = activeConversationId.value;
    if (previousConversationId && previousConversationId !== targetConversationId) {
      stashRuntimeState(
        runtimeStateByConversation,
        previousConversationId,
        activeRuntimeRefs,
        agentMode.value,
      );
    }

    activeConversationId.value = targetConversationId;
    const conversationMeta = conversations.value.find((c) => c.id === targetConversationId);
    activeWorkspacePath.value = conversationMeta?.workspacePath || '';
    planMode.value = agentMode.value === "plan";
    pendingUploads.value = [];

    try {
      const saved = await loadConversationHistory(targetConversationId);
      if (isStaleLoad()) return;
      const savedToolLogs = await loadConversationToolLogs(targetConversationId);
      if (isStaleLoad()) return;
      messages.value = (saved || [])
        .filter(
          (message) =>
            (message.role === "user" || message.role === "assistant") &&
            (!!message.content || !!message.reasoning || (message.attachments?.length ?? 0) > 0),
        )
        .map((message, index) => ({
          id:
            message.id != null && message.id > 0
              ? String(message.id)
              : `hist-${targetConversationId}-${index}`,
          role: message.role as "user" | "assistant",
          content: message.content,
          reasoning: message.reasoning,
          attachments: message.attachments,
          tokenUsage: message.tokenUsage,
          cost: message.cost,
          transcriptSegments: message.cost?.transcriptSegments,
        }));

      const restored = restoreRuntimeState(
        runtimeStateByConversation,
        targetConversationId,
        activeRuntimeRefs,
      );
      if (!restored || toolExecutionLogs.value.length === 0) {
        toolExecutionLogs.value = savedToolLogs;
      }
      await restoreLiveTurnStatus(targetConversationId);
      if (isStaleLoad()) return;

      await loadConversationMemory(targetConversationId);
      if (isStaleLoad()) return;
      await refreshConversationFiles(targetConversationId);
    } catch (err) {
      console.error("Failed to load conversation messages:", err);
      if (isStaleLoad()) return;
      messages.value = [];
      clearActiveRuntimeState(activeRuntimeRefs);
      conversationFiles.value = [];
    }
  }

  async function persistMessage(message: ChatMessage, conversationId = activeConversationId.value) {
    if (!conversationId) return;
    try {
      const dbId = await appendConversationMessage(conversationId, message);
      // 回写稳定数据库 id，编辑截断不再依赖易漂移的数组下标。
      if (dbId > 0) {
        const nextId = String(dbId);
        const idx = messages.value.findIndex(
          (item) => item === message || (!!message.id && item.id === message.id),
        );
        if (idx >= 0 && messages.value[idx]?.id !== nextId) {
          const next = messages.value.slice();
          next[idx] = { ...next[idx], id: nextId };
          messages.value = next;
        } else if (message.id !== nextId) {
          message.id = nextId;
        }
      }
      await refreshConversations();
    } catch (err) {
      console.error("Failed to persist message:", err);
    }
  }

  function clearAllSessionState() {
    clearActiveRuntimeState(activeRuntimeRefs);
    activeConversationId.value = "";
    activeWorkspacePath.value = "";
    messages.value = [];
    pendingUploads.value = [];
    conversationFiles.value = [];
    conversationMemory.value = null;
    toolExecutionLogs.value = [];
    planMode.value = agentMode.value === "plan";
  }

  async function handleNewChat() {
    if (isCreatingNewChat.value) return;

    // 已在空白欢迎界面：直接确保会话态彻底清理，无需重复创建流程
    if (!activeConversationId.value && messages.value.length === 0 && !hasConversationContent() && !assistantResponse.value.trim()) {
      clearAllSessionState();
      return;
    }

    const previousConversationId = activeConversationId.value;
    if (previousConversationId) {
      stashRuntimeState(
        runtimeStateByConversation,
        previousConversationId,
        activeRuntimeRefs,
        agentMode.value,
      );
    }

    isCreatingNewChat.value = true;
    try {
      // 不立即创建会话；让用户在欢迎页选择工作区，发消息时再创建。
      clearAllSessionState();
    } finally {
      isCreatingNewChat.value = false;
    }
  }

  async function handleSelectConversation(id: string) {
    if (!id || id === activeConversationId.value) return;
    await loadConversation(id);
  }

  async function handleDeleteConversation(id: string) {
    if (!id) return;
    if (
      isSpecificConversationGenerating(
        activeConversationId,
        isGenerating,
        runtimeStateByConversation,
        id,
      )
    ) {
      emitToast({
        variant: "info",
        source: "delete-conversation",
        message: "该会话正在回复中，请先停止后再删除。",
      });
      return;
    }

    const normalizedId = normalizeConversationId(id);
    runtimeStateByConversation.delete(normalizedId);
    void clearBrowserTabState(id);

    const isCurrentActive = activeConversationId.value === id;
    if (isCurrentActive) {
      // 提前清空 activeConversationId 和运行时会话态，
      // 避免随后 loadConversation 把被删除会话的状态重新 stash 进去。
      clearAllSessionState();
    }

    try {
      await deleteConversation(id);
      await refreshConversations();

      if (isCurrentActive) {
        if (conversations.value.length > 0) {
          await loadConversation(conversations.value[0].id);
        } else {
          clearAllSessionState();
        }
      }
    } catch (err) {
      console.error("Failed to delete conversation:", err);
    }
  }

  async function handlePinConversation(id: string, pinned: boolean) {
    if (!id) return;

    try {
      await setConversationPinned(id, pinned);
      await refreshConversations();
    } catch (err) {
      console.error("Failed to pin conversation:", err);
      emitToast({
        variant: "error",
        source: "pin-conversation",
        message: pinned ? "置顶会话失败。" : "取消置顶失败。",
      });
    }
  }

  const handleHistoryCleared = async () => {
    if (hasAnyGeneratingConversations(isGenerating, runtimeStateByConversation)) {
      emitToast({
        variant: "info",
        source: "history",
        message: "存在进行中的会话回复，请先停止后再清空历史。",
      });
      return;
    }

    runtimeStateByConversation.clear();
    clearAllSessionState();

    await refreshConversations();
    if (conversations.value.length === 0) {
      clearAllSessionState();
      return;
    }

    await loadConversation(conversations.value[0].id);
  };

  return {
    refreshConversationFiles,
    refreshActiveConversationFiles,
    loadConversationMemory,
    persistConversationMemory,
    refreshConversations,
    createNewConversation,
    loadConversation,
    restoreActiveLiveTurn,
    persistMessage,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation,
    handlePinConversation,
    handleHistoryCleared,
  };
}
