<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import WelcomeScreen from "@/components/chat/WelcomeScreen.vue";
import ChatScreen from "@/components/chat/ChatScreen.vue";
import WorkspaceDrawer from "@/components/chat/WorkspaceDrawer.vue";
import PlanPanel from "@/components/chat/PlanPanel.vue";
import BackgroundJobsPanel from "@/components/chat/BackgroundJobsPanel.vue";
import { useChatController } from "@/features/chat/controllers/useChatController";
import { useLayoutStore } from "@/stores/layout";
import { useConversationAgentBinding } from "@/features/agent/composables/useConversationAgentBinding";
import { useChatDropzone } from "@/composables/useChatDropzone";
import {
  BROWSER_ANNOTATION_SELECTED_EVENT,
  type BrowserAnnotationSelectedPayload,
} from "@/features/browser/browser-annotation";
import type { PendingUploadFile } from "@/lib/chat-types";

type BrowserOpenRequest = {
  conversationId?: string;
};

const route = useRoute();
const router = useRouter();
const layoutStore = useLayoutStore();
const chatController = useChatController();

const {
  messages,
  assistantTurnCost,
  toolExecutionLogs,
  currentTurnToolExecutionLogs,
  activeConversationId,
  activeWorkspacePath,
  conversationFiles,
  pendingAgentBundleId,
  chatScreenRef,
  handleSendMessage,
  handleEditMessage,
  handleLaunchAgentConversation,
  clearPendingAgent,
  handleUploadFiles,
  handleCancelGeneration,
  handlePendingQuestionSubmit,
  handlePendingQuestionSkip,
  handleSelectConversation,
  handleCompactConversation,
  dismissChatError,
} = chatController;

// 1. 会话绑定的智能体管理
const {
  displayAgent,
  removeConversationAgent,
} = useConversationAgentBinding({
  activeConversationId,
  pendingAgentBundleId,
  clearPendingAgent,
  handleLaunchAgentConversation,
});

// 2. 文件拖拽投放（在当前聊天视图下生效）
const chatViewRef = ref("chat");
const {
  isDraggingFiles,
  handleChatDragEnter,
  handleChatDragOver,
  handleChatDragLeave,
  handleChatDrop,
} = useChatDropzone({
  mainView: chatViewRef,
  onFilesAccepted: handleUploadFiles,
});

// 3. 路由参数联动：当 URL 包含 conversationId 时自动选中
watch(
  () => route.params.conversationId,
  async (targetId) => {
    if (typeof targetId === "string" && targetId.trim()) {
      const normalizedId = targetId.trim();
      if (normalizedId !== activeConversationId.value) {
        await handleSelectConversation(normalizedId);
      }
    }
  },
  { immediate: true },
);

const browserOpenRequestKey = ref(0);
let unlistenBrowserOpenRequest: UnlistenFn | null = null;
let unlistenBrowserAnnotationSelected: UnlistenFn | null = null;
let unlistenPlanUpdated: UnlistenFn | null = null;

const handleBrowserOpenRequest = async (payload: BrowserOpenRequest) => {
  const requestedConversationId = payload.conversationId?.trim();
  if (
    requestedConversationId &&
    requestedConversationId !== "__default__" &&
    requestedConversationId !== activeConversationId.value
  ) {
    await handleSelectConversation(requestedConversationId);
    void router.push(`/chat/${requestedConversationId}`);
  }

  layoutStore.activeWorkspaceTab = "browser";
  browserOpenRequestKey.value += 1;
};

const handleBrowserAnnotationSelected = async (payload: BrowserAnnotationSelectedPayload) => {
  const requestedConversationId = payload.conversationId?.trim();
  if (
    requestedConversationId &&
    requestedConversationId !== "__default__" &&
    requestedConversationId !== activeConversationId.value
  ) {
    await handleSelectConversation(requestedConversationId);
    void router.push(`/chat/${requestedConversationId}`);
  }

  const content = payload.content?.trim();
  if (!content) return;

  const file: PendingUploadFile = {
    kind: "document",
    sourceName: payload.sourceName || "浏览器注释.md",
    mimeType: "text/markdown",
    content,
    rawBytes: null,
    size: new TextEncoder().encode(content).length,
  };
  await handleUploadFiles([file]);
};

onMounted(() => {
  void listen<BrowserOpenRequest>("nova-browser-open-request", (event) => {
    void handleBrowserOpenRequest(event.payload);
  }).then((unlisten) => {
    unlistenBrowserOpenRequest = unlisten;
  }).catch((error) => {
    console.warn("Browser open request listener failed:", error);
  });

  void listen<BrowserAnnotationSelectedPayload>(BROWSER_ANNOTATION_SELECTED_EVENT, (event) => {
    void handleBrowserAnnotationSelected(event.payload);
  }).then((unlisten) => {
    unlistenBrowserAnnotationSelected = unlisten;
  }).catch((error) => {
    console.warn("Browser annotation listener failed:", error);
  });

  void listen<{ conversationId?: string | null }>("plan-updated", (event) => {
    const payloadConversationId = event.payload?.conversationId?.trim();
    if (
      payloadConversationId &&
      payloadConversationId !== "__default__" &&
      payloadConversationId !== activeConversationId.value
    ) {
      return;
    }
    layoutStore.isPlanPanelOpen = true;
  }).then((unlisten) => {
    unlistenPlanUpdated = unlisten;
  }).catch((error) => {
    console.warn("Plan updated listener failed:", error);
  });
});

onBeforeUnmount(() => {
  unlistenBrowserOpenRequest?.();
  unlistenBrowserAnnotationSelected?.();
  unlistenPlanUpdated?.();
});
</script>

<template>
  <div class="relative flex h-full min-w-0 flex-1 overflow-hidden">
    <!-- Chat 主交互面板 -->
    <section
      class="app-chat-pane relative flex h-full min-w-0 flex-1 flex-col overflow-hidden"
      @dragenter="handleChatDragEnter"
      @dragover="handleChatDragOver"
      @dragleave="handleChatDragLeave"
      @drop="handleChatDrop"
    >
      <WelcomeScreen
        v-if="messages.length === 0"
        :workspacePath="activeWorkspacePath"
        :conversationId="activeConversationId"
        :activeAgent="displayAgent"
        @update:workspacePath="activeWorkspacePath = $event"
        @send="handleSendMessage"
        @remove-agent="removeConversationAgent"
      />

      <ChatScreen
        v-else
        ref="chatScreenRef"
        :activeAgent="displayAgent"
        :drawerOpen="layoutStore.isDrawerOpen"
        @open-plan="layoutStore.isPlanPanelOpen = true"
        @open-background-jobs="layoutStore.isBgJobsPanelOpen = true"
        @remove-agent="removeConversationAgent"
        @send="handleSendMessage"
        @save-user-edit="handleEditMessage($event)"
        @cancel="handleCancelGeneration"
        @ask-submit="handlePendingQuestionSubmit"
        @ask-skip="handlePendingQuestionSkip"
        @compact="handleCompactConversation"
        @dismiss-error="dismissChatError"
      />

      <!-- 拖拽文件悬停提示层：pointer-events-none 保证 drop 仍落在面板上 -->
      <div
        v-if="isDraggingFiles"
        class="pointer-events-none absolute inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px]"
      >
        <div class="flex items-center gap-2.5 rounded-xl border-2 border-dashed border-white/80 px-6 py-4 text-[15px] font-medium text-white">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
            <polyline points="17 8 12 3 7 8" />
            <line x1="12" y1="3" x2="12" y2="15" />
          </svg>
          松开以添加文件到对话
        </div>
      </div>
    </section>

    <!-- 计划侧边面板：独立于工作区抽屉，只在有计划交互时弹出 -->
    <PlanPanel
      :open="layoutStore.isPlanPanelOpen"
      :conversationId="activeConversationId || null"
      @close="layoutStore.isPlanPanelOpen = false"
    />

    <BackgroundJobsPanel
      :open="layoutStore.isBgJobsPanelOpen"
      :conversationId="activeConversationId || null"
      @close="layoutStore.isBgJobsPanelOpen = false"
    />

    <WorkspaceDrawer
      :open="layoutStore.isDrawerOpen"
      :activeTab="layoutStore.activeWorkspaceTab"
      :entries="toolExecutionLogs"
      :currentTurnToolEntries="currentTurnToolExecutionLogs"
      :messages="messages"
      :files="conversationFiles"
      :assistantTurnCost="assistantTurnCost"
      :conversationId="activeConversationId || null"
      :browserOpenRequestKey="browserOpenRequestKey"
      :width="layoutStore.drawerWidth"
      @close="layoutStore.isDrawerOpen = false"
      @resize="layoutStore.handleDrawerResize"
      @resize-end="layoutStore.handleDrawerResizeEnd"
    />
  </div>
</template>
