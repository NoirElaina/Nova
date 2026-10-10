import { ref, type Ref } from "vue";
import { emitToast } from "../../../lib/toast";
import type {
  AgentMode,
  ChatMessage,
  ConversationMeta,
} from "../../../lib/chat-types";
import {
  appendPlainChatMessage,
  createConversation,
  deleteConversation,
  listSessionFiles,
  listConversations,
  setConversationPinned,
  type SessionFileMeta,
} from "../services/chat-api";
import { clearBrowserTabState } from "../../browser/browser-tab-state";
import { clearSubagents } from "../services/subagents";
import { isScheduledConversationTitle } from "./chat-controller-types";
import { useAgentSessionStore } from "@/stores/agentSession";
import { useComposerStore } from "@/stores/composer";

export type ConversationOpsDeps = {
  activeConversationId: Ref<string>;
  activeWorkspacePath: Ref<string>;
  agentMode: Ref<AgentMode>;
  conversations: Ref<ConversationMeta[]>;
  conversationFiles: Ref<SessionFileMeta[]>;
  hasConversationContent: () => boolean;
};

export function createConversationOperations(deps: ConversationOpsDeps) {
  const {
    activeConversationId,
    activeWorkspacePath,
    conversations,
    conversationFiles,
    hasConversationContent,
  } = deps;

  const sessionStore = useAgentSessionStore();
  const composerStore = useComposerStore();
  const isCreatingNewChat = ref(false);

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

  async function refreshConversations() {
    try {
      const items = await listConversations();
      conversations.value = (items || []).filter(
        (item) => !isScheduledConversationTitle(item.title),
      );
    } catch (err) {
      console.error("Failed to list conversations:", err);
    }
  }

  async function createNewConversation(seedTitle?: string): Promise<string | null> {
    try {
      const conv = await createConversation(seedTitle, activeWorkspacePath.value || undefined);
      activeWorkspacePath.value = conv.workspacePath || "";
      await refreshConversations();
      return conv.id;
    } catch (err) {
      console.error("Failed to create conversation:", err);
      return null;
    }
  }

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

    activeConversationId.value = targetConversationId;
    const conversationMeta = conversations.value.find((c) => c.id === targetConversationId);
    activeWorkspacePath.value = conversationMeta?.workspacePath || "";
    composerStore.clearUploads();

    try {
      await sessionStore.loadConversationMessages(targetConversationId);
      if (isStaleLoad()) return;

      await refreshConversationFiles(targetConversationId);
    } catch (err) {
      console.error("Failed to load conversation messages:", err);
      if (isStaleLoad()) return;
      conversationFiles.value = [];
    }
  }

  async function persistMessage(message?: ChatMessage, conversationId = activeConversationId.value) {
    if (!conversationId || !message) {
      return;
    }
    try {
      await appendPlainChatMessage(
        conversationId,
        message.role === "assistant" ? "assistant" : "user",
        message.content,
      );
      await refreshConversations();
    } catch (err) {
      console.error("Failed to persist message:", err);
    }
  }

  function clearAllSessionState() {
    activeConversationId.value = "";
    activeWorkspacePath.value = "";
    composerStore.clearUploads();
    conversationFiles.value = [];
    sessionStore.clearActiveTurnRuntime();
  }

  async function handleNewChat() {
    if (isCreatingNewChat.value) return;

    if (!activeConversationId.value && sessionStore.activeSession.messages.length === 0 && !hasConversationContent()) {
      clearAllSessionState();
      return;
    }

    isCreatingNewChat.value = true;
    try {
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
    if (sessionStore.isConversationGenerating(id)) {
      emitToast({
        variant: "info",
        source: "delete-conversation",
        message: "该会话正在回复中，请先停止后再删除。",
      });
      return;
    }

    sessionStore.deleteSession(id);
    void clearBrowserTabState(id);

    const isCurrentActive = activeConversationId.value === id;
    if (isCurrentActive) {
      clearAllSessionState();
    }

    try {
      await deleteConversation(id);
      clearSubagents(id);
      await refreshConversations();

      if (isCurrentActive) {
        const remaining = conversations.value.filter((item) => item.id !== id);
        if (remaining.length > 0) {
          await loadConversation(remaining[0].id);
        } else {
          clearAllSessionState();
        }
      }
    } catch (err) {
      console.error("Failed to delete conversation:", err);
      emitToast({
        variant: "error",
        source: "delete-conversation",
        message: "删除会话失败，请重试。",
      });
    }
  }

  async function handlePinConversation(id: string, pinned: boolean) {
    try {
      await setConversationPinned(id, pinned);
      await refreshConversations();
    } catch (err) {
      console.error("Failed to toggle pin for conversation:", err);
      emitToast({
        variant: "error",
        source: "pin-conversation",
        message: "固定/取消固定会话失败，请重试。",
      });
    }
  }

  async function handleHistoryCleared(event: CustomEvent<{ conversationId?: string }>) {
    const clearedId = event.detail?.conversationId;
    if (sessionStore.hasAnyGenerating) {
      emitToast({
        variant: "warning",
        source: "clear-history",
        message: "会话正在生成中，已在后台清空历史，生成结束后生效。",
      });
      return;
    }

    if (!clearedId || clearedId === activeConversationId.value) {
      if (activeConversationId.value) {
        await sessionStore.loadConversationMessages(activeConversationId.value);
      }
      sessionStore.clearActiveTurnRuntime();
    }
    await refreshConversations();
  }

  return {
    refreshConversationFiles,
    refreshActiveConversationFiles,
    refreshConversations,
    createNewConversation,
    loadConversation,
    persistMessage,
    handleNewChat,
    handleSelectConversation,
    handleDeleteConversation,
    handlePinConversation,
    handleHistoryCleared,
  };
}
