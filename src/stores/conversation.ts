import { defineStore } from "pinia";
import { ref } from "vue";
import type { ConversationMeta } from "@/lib/chat-types";
import {
  listConversations,
  deleteConversation as apiDeleteConversation,
  setConversationPinned as apiSetConversationPinned,
  listSessionFiles,
  type SessionFileMeta,
} from "@/features/chat/services/chat-api";

export type MainView = "chat" | "hooks" | "agent" | "schedule" | "settings";

export const useConversationStore = defineStore("conversation", () => {
  const conversations = ref<ConversationMeta[]>([]);
  const activeConversationId = ref<string>("");
  const activeWorkspacePath = ref<string>("");
  const conversationFiles = ref<SessionFileMeta[]>([]);
  const isSidebarOpen = ref<boolean>(true);
  const mainView = ref<MainView>("chat");
  const pendingAgentBundleId = ref<string | null>(null);

  async function loadConversations() {
    try {
      const items = await listConversations();
      conversations.value = (items || []).filter(
        (item) => !item.title.startsWith("Scheduled ["),
      );
    } catch (err) {
      console.error("Failed to load conversations:", err);
      conversations.value = [];
    }
  }

  async function refreshConversationFiles(convId?: string) {
    const id = convId ?? activeConversationId.value;
    if (!id) {
      conversationFiles.value = [];
      return;
    }
    try {
      conversationFiles.value = await listSessionFiles(id);
    } catch (err) {
      console.error("Failed to load session files:", err);
      conversationFiles.value = [];
    }
  }

  function selectConversation(id: string) {
    activeConversationId.value = id;
    mainView.value = "chat";
    pendingAgentBundleId.value = null;
    void refreshConversationFiles(id);
  }

  function newChat() {
    activeConversationId.value = "";
    mainView.value = "chat";
    pendingAgentBundleId.value = null;
    conversationFiles.value = [];
  }

  async function deleteConversation(id: string) {
    await apiDeleteConversation(id);
    conversations.value = conversations.value.filter((c) => c.id !== id);
    if (activeConversationId.value === id) {
      newChat();
    }
  }

  async function pinConversation(id: string, pinned: boolean) {
    await apiSetConversationPinned(id, pinned);
    const item = conversations.value.find((c) => c.id === id);
    if (item) {
      item.pinnedAt = pinned ? Date.now() : null;
    }
  }

  function setMainView(view: MainView) {
    mainView.value = view;
  }

  function setWorkspacePath(path: string) {
    activeWorkspacePath.value = path;
  }

  return {
    conversations,
    activeConversationId,
    activeWorkspacePath,
    conversationFiles,
    isSidebarOpen,
    mainView,
    pendingAgentBundleId,
    loadConversations,
    refreshConversationFiles,
    selectConversation,
    newChat,
    deleteConversation,
    pinConversation,
    setMainView,
    setWorkspacePath,
  };
});
