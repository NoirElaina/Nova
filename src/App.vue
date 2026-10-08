<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import Sidebar from "./components/layout/Sidebar.vue";
import WelcomeScreen from "./components/chat/WelcomeScreen.vue";
import ChatScreen from "./components/chat/ChatScreen.vue";
import WorkspaceDrawer from "./components/chat/WorkspaceDrawer.vue";
import PlanPanel from "./components/chat/PlanPanel.vue";
import BackgroundJobsPanel from "./components/chat/BackgroundJobsPanel.vue";
import HooksConfigScreen from "./components/hooks/HooksConfigScreen.vue";
import AgentConfigScreen from "./components/agent/AgentConfigScreen.vue";
import ScheduleTaskScreen from "./components/schedule/ScheduleTaskScreen.vue";
import SettingsScreen from "./components/settings/SettingsScreen.vue";
import GlobalToastHost from "./components/layout/GlobalToastHost.vue";
import { useChatController } from "./features/chat/controllers/useChatController";
import { useLayoutStore } from "./stores/layout";
import { useConversationAgentBinding } from "./features/agent/composables/useConversationAgentBinding";
import { useConversationExport } from "./features/chat/composables/useConversationExport";
import { useChatDropzone } from "./composables/useChatDropzone";
import {
  BROWSER_ANNOTATION_SELECTED_EVENT,
  type BrowserAnnotationSelectedPayload,
} from "./features/browser/browser-annotation";
import type { PendingUploadFile } from "./lib/chat-types";

type BrowserOpenRequest = {
  conversationId?: string;
};

const layoutStore = useLayoutStore();

const {
  messages,
  assistantTurnCost,
  toolExecutionLogs,
  currentTurnToolExecutionLogs,
  conversations,
  activeConversationId,
  activeWorkspacePath,
  conversationFiles,
  mainView,
  isSidebarOpen,
  chatScreenRef,
  handleSendMessage,
  handleEditMessage,
  handleLaunchAgentConversation,
  clearPendingAgent,
  pendingAgentBundleId,
  handleUploadFiles,
  handleCancelGeneration,
  handlePendingQuestionSubmit,
  handlePendingQuestionSkip,
  handleNewChat,
  handleSelectConversation,
  handleDeleteConversation,
  handlePinConversation,
  handleChangeMainView,
  handleCompactConversation,
  dismissChatError,
} = useChatController();

// 1. 会话绑定的智能体管理
const {
  displayAgent,
  removeConversationAgent,
  handleLaunchAgent,
} = useConversationAgentBinding({
  activeConversationId,
  pendingAgentBundleId,
  clearPendingAgent,
  handleLaunchAgentConversation,
});

// 2. 会话导出管理
const {
  exportingConversationId,
  exportingFormat,
  handleExportConversation,
} = useConversationExport(conversations);

// 3. 文件拖拽投放
const {
  isDraggingFiles,
  handleChatDragEnter,
  handleChatDragOver,
  handleChatDragLeave,
  handleChatDrop,
} = useChatDropzone({
  mainView,
  onFilesAccepted: handleUploadFiles,
});

const activeWorkspaceName = computed(() => {
  const path = activeWorkspacePath.value?.trim();
  if (!path) return '';
  const parts = path.replace(/\\/g, '/').split('/');
  const last = parts[parts.length - 1] || '';
  if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(last)) {
    return '默认工作区';
  }
  return last;
});

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
  }

  handleChangeMainView("chat");
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
  }

  const content = payload.content?.trim();
  if (!content) return;

  handleChangeMainView("chat");
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
  const preventDefaultDrag = (event: DragEvent) => {
    if (Array.from(event.dataTransfer?.types ?? []).includes("Files")) {
      event.preventDefault();
    }
  };
  window.addEventListener("dragover", preventDefaultDrag);
  window.addEventListener("drop", preventDefaultDrag);

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
    if (mainView.value !== "chat") return;
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
  <div class="flex h-screen bg-[#fcfcfc] dark:bg-[#1a1a1a] text-[#1a1a1a] dark:text-[#ececec] overflow-hidden font-sans">
    <GlobalToastHost />

    <SettingsScreen
      v-if="mainView === 'settings'"
      @change-main-view="handleChangeMainView"
    />

    <template v-else>
      <Sidebar
        v-if="isSidebarOpen"
        :recents="conversations"
        :activeConversationId="activeConversationId"
        :activeMainView="mainView"
        :exportingConversationId="exportingConversationId"
        :exportingFormat="exportingFormat"
        :width="layoutStore.sidebarWidth"
        @new-chat="handleNewChat"
        @select-conversation="handleSelectConversation"
        @delete-conversation="handleDeleteConversation"
        @pin-conversation="handlePinConversation"
        @export-conversation="handleExportConversation"
        @change-main-view="handleChangeMainView"
        @toggle-sidebar="isSidebarOpen = !isSidebarOpen"
        @resize="layoutStore.handleSidebarResize"
        @resize-end="layoutStore.handleSidebarResizeEnd"
      />

      <!-- Main Content Area -->
      <main class="relative flex h-full min-w-0 flex-1 overflow-hidden">
        <section
          class="app-chat-pane relative flex h-full min-w-0 flex-1 flex-col overflow-hidden"
          @dragenter="handleChatDragEnter"
          @dragover="handleChatDragOver"
          @dragleave="handleChatDragLeave"
          @drop="handleChatDrop"
        >
          <!-- Top Title Bar -->
          <header class="h-14 flex items-center justify-between px-4 absolute top-0 w-full z-10 pointer-events-none">
            <div class="flex items-center gap-2 pointer-events-auto">
              <Button
                variant="ghost"
                size="icon-sm"
                class="h-8 w-8 text-muted-foreground hover:bg-black/5 dark:hover:bg-white/5"
                @click="isSidebarOpen = !isSidebarOpen"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><line x1="9" y1="3" x2="9" y2="21"/></svg>
              </Button>
              <span
                v-if="activeWorkspacePath"
                class="text-[12px] text-[#64748b] dark:text-[#9ca3af] truncate max-w-[260px]"
                :title="activeWorkspacePath"
              >{{ activeWorkspaceName }}</span>
            </div>

            <div v-if="mainView === 'chat'" class="flex items-center gap-2 pointer-events-auto">
              <Button
                variant="ghost"
                size="icon-sm"
                class="h-8 w-8 rounded-md text-[#4f5f73] hover:bg-black/5 dark:text-[#d5dbe3] dark:hover:bg-white/5"
                :class="{ 'bg-black/5 dark:bg-white/10': layoutStore.isDrawerOpen }"
                title="工作区面板"
                @click="layoutStore.isDrawerOpen = !layoutStore.isDrawerOpen"
              >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="3" y="3" width="18" height="18" rx="2"/>
                  <line x1="15" y1="3" x2="15" y2="21"/>
                </svg>
              </Button>
            </div>
          </header>

          <HooksConfigScreen
            v-if="mainView === 'hooks'"
            @change-main-view="handleChangeMainView"
          />

          <AgentConfigScreen
            v-else-if="mainView === 'agent'"
            :conversation-id="activeConversationId || null"
            @change-main-view="handleChangeMainView"
            @launch-agent="handleLaunchAgent"
          />

          <ScheduleTaskScreen
            v-else-if="mainView === 'schedule'"
            @change-main-view="handleChangeMainView"
            @open-task-conversation="handleSelectConversation"
          />

          <template v-else>
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
          </template>

          <!-- 拖拽文件悬停提示层：pointer-events-none 保证 drop 仍落在面板上 -->
          <div
            v-if="isDraggingFiles && mainView === 'chat'"
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
          v-if="mainView === 'chat'"
          :open="layoutStore.isPlanPanelOpen"
          :conversationId="activeConversationId || null"
          @close="layoutStore.isPlanPanelOpen = false"
        />

        <BackgroundJobsPanel
          v-if="mainView === 'chat'"
          :open="layoutStore.isBgJobsPanelOpen"
          :conversationId="activeConversationId || null"
          @close="layoutStore.isBgJobsPanelOpen = false"
        />

        <WorkspaceDrawer
          v-if="mainView === 'chat'"
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
      </main>
    </template>
  </div>
</template>

<style>
html, body, #app {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
}
</style>
