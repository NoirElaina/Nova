import type { Ref } from "vue";
import { emitToast } from "../../../lib/toast";
import type {
  AgentMode,
  ChatMessage,
  ConversationMemory,
  ConversationMeta,
  PendingUploadFile,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";
import {
  appendConversationMessage,
  createConversation,
  deleteConversation,
  getConversationMemory,
  listConversationRagDocuments,
  listConversations,
  loadConversationHistory,
  loadConversationToolLogs,
  type RagDocumentMeta,
  upsertConversationMemory,
} from "../services/chat-api";
import { extractSessionMemory } from "../utils/session-memory";
import type { ConversationTurnRuntimeState } from "./chat-controller-types";
import type { ActiveRuntimeRefs } from "./chat-runtime-state";
import {
  clearActiveRuntimeState,
  hasAnyGeneratingConversations,
  isSpecificConversationGenerating,
  normalizeConversationId,
  resetTurnRuntimeState,
  restoreRuntimeState,
  stashRuntimeState,
} from "./chat-runtime-state";

type ConversationOpsDeps = {
  activeConversationId: Ref<string>;
  agentMode: Ref<AgentMode>;
  planMode: Ref<boolean>;
  isGenerating: Ref<boolean>;
  isCreatingNewChat: Ref<boolean>;
  conversations: Ref<ConversationMeta[]>;
  messages: Ref<ChatMessage[]>;
  toolExecutionLogs: Ref<ToolExecutionEntry[]>;
  conversationFiles: Ref<RagDocumentMeta[]>;
  pendingUploads: Ref<PendingUploadFile[]>;
  conversationMemory: Ref<ConversationMemory | null>;
  assistantResponse: Ref<string>;
  assistantReasoning: Ref<string>;
  assistantTokenUsage: Ref<number | undefined>;
  assistantTurnCost: Ref<TurnCost | undefined>;
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>;
  activeRuntimeRefs: ActiveRuntimeRefs;
  hasConversationContent: () => boolean;
};

export function createConversationOperations(deps: ConversationOpsDeps) {
  const {
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
  } = deps;

  async function refreshConversationFiles(conversationId: string) {
    if (!conversationId) {
      conversationFiles.value = [];
      return;
    }

    try {
      conversationFiles.value = await listConversationRagDocuments(conversationId);
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
      const conv = await createConversation(seedTitle);
      await refreshConversations();
      return conv.id;
    } catch (err) {
      console.error("Failed to create conversation:", err);
      return null;
    }
  }

  async function loadConversation(id: string) {
    const targetConversationId = id.trim();
    if (!targetConversationId) {
      return;
    }

    const previousConversationId = activeConversationId.value;
    if (previousConversationId && previousConversationId !== targetConversationId) {
      stashRuntimeState(runtimeStateByConversation, previousConversationId, activeRuntimeRefs);
    }

    activeConversationId.value = targetConversationId;
    planMode.value = agentMode.value === "plan";
    pendingUploads.value = [];

    try {
      const saved = await loadConversationHistory(targetConversationId);
      const savedToolLogs = await loadConversationToolLogs(targetConversationId);
      messages.value = (saved || [])
        .filter(
          (message) =>
            (message.role === "user" || message.role === "assistant") &&
            (!!message.content || !!message.reasoning || (message.attachments?.length ?? 0) > 0),
        )
        .map((message) => ({
          role: message.role as "user" | "assistant",
          content: message.content,
          reasoning: message.reasoning,
          attachments: message.attachments,
          tokenUsage: message.tokenUsage,
          cost: message.cost,
        }));

      const restored = restoreRuntimeState(
        runtimeStateByConversation,
        targetConversationId,
        activeRuntimeRefs,
      );
      if (!restored || toolExecutionLogs.value.length === 0) {
        toolExecutionLogs.value = savedToolLogs;
      }

      await loadConversationMemory(targetConversationId);
      await refreshConversationFiles(targetConversationId);
    } catch (err) {
      console.error("Failed to load conversation messages:", err);
      messages.value = [];
      clearActiveRuntimeState(activeRuntimeRefs);
      conversationFiles.value = [];
    }
  }

  async function persistMessage(message: ChatMessage, conversationId = activeConversationId.value) {
    if (!conversationId) return;
    try {
      await appendConversationMessage(conversationId, message);
      await refreshConversations();
    } catch (err) {
      console.error("Failed to persist message:", err);
    }
  }

  async function handleNewChat() {
    if (isCreatingNewChat.value) return;

    resetTurnRuntimeState(activeRuntimeRefs);

    if (activeConversationId.value && !hasConversationContent() && !assistantResponse.value.trim()) {
      return;
    }

    isCreatingNewChat.value = true;
    try {
      const id = await createNewConversation("New chat");
      if (!id) {
        return;
      }

      await loadConversation(id);
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

    runtimeStateByConversation.delete(normalizeConversationId(id));
    try {
      await deleteConversation(id);
      await refreshConversations();

      if (activeConversationId.value === id) {
        if (conversations.value.length > 0) {
          await loadConversation(conversations.value[0].id);
        } else {
          const newId = await createNewConversation("New chat");
          if (newId) {
            await loadConversation(newId);
          } else {
            activeConversationId.value = "";
            messages.value = [];
            pendingUploads.value = [];
            conversationFiles.value = [];
          }
        }
      }
    } catch (err) {
      console.error("Failed to delete conversation:", err);
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
    resetTurnRuntimeState(activeRuntimeRefs);
    assistantResponse.value = "";
    assistantReasoning.value = "";
    assistantTokenUsage.value = undefined;
    assistantTurnCost.value = undefined;
    pendingUploads.value = [];
    toolExecutionLogs.value = [];
    conversationFiles.value = [];
    conversationMemory.value = null;
    messages.value = [];

    await refreshConversations();
    if (conversations.value.length === 0) {
      const newId = await createNewConversation("New chat");
      if (newId) {
        await loadConversation(newId);
      } else {
        activeConversationId.value = "";
      }
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
    persistMessage,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation,
    handleHistoryCleared,
  };
}
