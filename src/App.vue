<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import Sidebar from "./components/layout/Sidebar.vue";
import WelcomeScreen from "./components/chat/WelcomeScreen.vue";
import ChatScreen from "./components/chat/ChatScreen.vue";
import ExecutionTracePopover from "./components/chat/files/ExecutionTracePopover.vue";
import TodoProgressPopover from "./components/chat/files/TodoProgressPopover.vue";
import WorkspaceDrawer from "./components/chat/WorkspaceDrawer.vue";
import HooksConfigScreen from "./components/hooks/HooksConfigScreen.vue";
import AgentConfigScreen from "./components/agent/AgentConfigScreen.vue";
import AgentMarketScreen from "./components/agent/AgentMarketScreen.vue";
import ScheduleTaskScreen from "./components/schedule/ScheduleTaskScreen.vue";
import SettingsScreen from "./components/layout/settings/SettingsScreen.vue";
import GlobalToastHost from "./components/layout/GlobalToastHost.vue";
import { useChatController } from "./features/chat/controllers/useChatController";
import {
  BROWSER_ANNOTATION_SELECTED_EVENT,
  type BrowserAnnotationSelectedPayload,
} from "./features/browser/browser-annotation";
import type { PendingUploadFile } from "./lib/chat-types";
import { buildPendingUploadFiles, notifyRejectedUploads } from "./lib/upload-files";
import {
  exportConversation,
  exportRenderedConversationPdf,
  loadConversationHistory,
  type ConversationExportFormat,
} from "./features/chat/services/chat-api";
import { buildConversationExportHtml } from "./features/chat/utils/conversation-export-html";
import { emitToast } from "./lib/toast";
import {
  getStoredSidebarWidth,
  setStoredSidebarWidth,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_MAX_WIDTH,
  getStoredDrawerWidth,
  setStoredDrawerWidth,
  clampDrawerWidth,
} from "./lib/ui-preferences";

type WorkspaceTabId = "workspace" | "plan" | "diff" | "usage" | "files" | "terminal" | "browser";
type BrowserOpenRequest = {
  conversationId?: string;
};

const {
  messages,
  isGenerating,
  currentStage,
  assistantResponse,
  assistantReasoning,
  assistantSegments,
  assistantTokenUsage,
  assistantTurnCost,
  toolExecutionLogs,
  currentTurnToolExecutionLogs,
  conversations,
  activeConversationId,
  activeWorkspacePath,
  conversationFiles,
  pendingUploads,
  currentContextUsage,
  currentContextCompacts,
  currentContextTokens,
  pendingQuestion,
  pendingPermissionRequestId,
  agentMode,
  planMode,
  mainView,
  isSidebarOpen,
  chatScreenRef,
  handleSendMessage,
  handleEditMessage,
  handleUploadFiles,
  handleRemovePendingUpload,
  handleCancelGeneration,
  handlePendingQuestionSubmit,
  handlePendingQuestionSkip,
  handleAgentModeChange,
  handleNewChat,
  handleSelectConversation,
  handleDeleteConversation,
  handlePinConversation,
  handleChangeMainView,
  isCompacting,
  handleCompactConversation,
} = useChatController();

void chatScreenRef;

const activeWorkspaceName = computed(() => {
  const path = activeWorkspacePath.value?.trim();
  if (!path) return '';
  const parts = path.replace(/\\/g, '/').split('/');
  const last = parts[parts.length - 1] || '';
  // 默认工作区目录名是会话 uuid，直接显示太长；换成友好名称。
  if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(last)) {
    return '默认工作区';
  }
  return last;
});


const isDrawerOpen = ref(false);
const activeWorkspaceTab = ref<WorkspaceTabId>("workspace");
const browserOpenRequestKey = ref(0);

// 侧边栏宽度：拖动时实时更新，松手后持久化到 localStorage。
const sidebarWidth = ref(getStoredSidebarWidth());
const handleSidebarResize = (width: number) => {
  sidebarWidth.value = Math.min(Math.max(width, SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
};
const handleSidebarResizeEnd = () => {
  setStoredSidebarWidth(sidebarWidth.value);
};

// 工作区抽屉宽度：同样拖动实时生效、松手持久化。
const drawerWidth = ref(getStoredDrawerWidth());
const handleDrawerResize = (width: number) => {
  drawerWidth.value = clampDrawerWidth(width);
};
const handleDrawerResizeEnd = () => {
  setStoredDrawerWidth(drawerWidth.value);
};
const exportingConversationId = ref<string | null>(null);
const exportingFormat = ref<ConversationExportFormat | null>(null);
let unlistenBrowserOpenRequest: UnlistenFn | null = null;
let unlistenBrowserAnnotationSelected: UnlistenFn | null = null;
let unlistenPlanUpdated: UnlistenFn | null = null;

// 拖拽文件到聊天面板直接导入。dragenter/dragleave 会在子元素间频繁冒泡，
// 用计数器而不是布尔值判断"是否仍在拖拽悬停"。
const isDraggingFiles = ref(false);
let dragDepth = 0;

const hasDraggedFiles = (event: DragEvent) =>
  Array.from(event.dataTransfer?.types ?? []).includes("Files");

const handleChatDragEnter = (event: DragEvent) => {
  if (mainView.value !== 'chat' || !hasDraggedFiles(event)) return;
  dragDepth += 1;
  isDraggingFiles.value = true;
};

const handleChatDragOver = (event: DragEvent) => {
  if (!hasDraggedFiles(event)) return;
  event.preventDefault();
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = "copy";
  }
};

const handleChatDragLeave = (event: DragEvent) => {
  if (!hasDraggedFiles(event)) return;
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) {
    isDraggingFiles.value = false;
  }
};

const handleChatDrop = async (event: DragEvent) => {
  if (mainView.value !== 'chat' || !hasDraggedFiles(event)) return;
  event.preventDefault();
  dragDepth = 0;
  isDraggingFiles.value = false;

  const files = Array.from(event.dataTransfer?.files ?? []);
  if (files.length === 0) return;

  const { accepted, rejected } = await buildPendingUploadFiles(files);
  if (accepted.length > 0) {
    await handleUploadFiles(accepted);
  }
  notifyRejectedUploads(rejected);
};

const formatExportLabel = (format: ConversationExportFormat) => format.toUpperCase();

const handleExportConversation = async (
  conversationId: string,
  format: ConversationExportFormat,
) => {
  if (exportingConversationId.value) {
    return;
  }

  exportingConversationId.value = conversationId;
  exportingFormat.value = format;

  try {
    const conversation = conversations.value.find((item) => item.id === conversationId);
    const title = conversation?.title || "New chat";
    const exportPath =
      format === "pdf"
        ? await exportRenderedConversationPdf(
            conversationId,
            title,
            buildConversationExportHtml({
              conversationId,
              title,
              exportedAt: new Date().toISOString(),
              messages: await loadConversationHistory(conversationId),
            }),
          )
        : await exportConversation(conversationId, "json");
    emitToast({
      variant: "success",
      source: "conversation-export",
      message: `${formatExportLabel(format)} 已导出到：${exportPath}`,
    });
  } catch (err) {
    console.error("Failed to export conversation:", err);
    emitToast({
      variant: "error",
      source: "conversation-export",
      message: `导出 ${formatExportLabel(format)} 失败。`,
    });
  } finally {
    exportingConversationId.value = null;
    exportingFormat.value = null;
  }
};

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
  activeWorkspaceTab.value = "browser";
  isDrawerOpen.value = true;
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
  // 阻止 webview 默认行为：在非拖拽区松手会直接导航打开该文件，把整个应用界面替换掉。
  const preventDefaultDrag = (event: DragEvent) => {
    if (!hasDraggedFiles(event)) return;
    event.preventDefault();
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

  // exit_plan_mode 保存 plan 后自动打开抽屉的「计划」页，让计划直接可见。
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
    activeWorkspaceTab.value = "plan";
    isDrawerOpen.value = true;
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
        :width="sidebarWidth"
        @new-chat="handleNewChat"
        @select-conversation="handleSelectConversation"
        @delete-conversation="handleDeleteConversation"
        @pin-conversation="handlePinConversation"
        @export-conversation="handleExportConversation"
        @change-main-view="handleChangeMainView"
        @toggle-sidebar="isSidebarOpen = !isSidebarOpen"
        @resize="handleSidebarResize"
        @resize-end="handleSidebarResizeEnd"
      />

      <!-- Main Content Area -->
      <main class="relative flex h-full min-w-0 flex-1 overflow-hidden">
      <section
        class="app-chat-pane relative flex h-full min-w-0 flex-1 flex-col"
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
            <TodoProgressPopover :conversation-id="activeConversationId" />
            <ExecutionTracePopover :entries="toolExecutionLogs" />
            <Button
              variant="ghost"
              size="icon-sm"
              class="h-8 w-8 rounded-md text-[#4f5f73] hover:bg-black/5 dark:text-[#d5dbe3] dark:hover:bg-white/5"
              :class="{ 'bg-black/5 dark:bg-white/10': isDrawerOpen }"
              title="工作区面板"
              @click="isDrawerOpen = !isDrawerOpen"
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
          @change-main-view="handleChangeMainView"
        />

        <AgentMarketScreen
          v-else-if="mainView === 'agentMarket'"
        />

        <ScheduleTaskScreen
          v-else-if="mainView === 'schedule'"
          @change-main-view="handleChangeMainView"
          @open-task-conversation="handleSelectConversation"
        />

        <template v-else>
          <WelcomeScreen
            v-if="messages.length === 0"
            :isGenerating="isGenerating"
            :agentMode="agentMode"
            :pendingUploads="pendingUploads"
            :contextUsage="currentContextUsage"
            :contextCompacts="currentContextCompacts"
            :contextTokens="currentContextTokens"
            :workspacePath="activeWorkspacePath"
            :conversationId="activeConversationId"
            @update:workspacePath="activeWorkspacePath = $event"
            @send="handleSendMessage"
            @mode-change="handleAgentModeChange"
            @upload-files="handleUploadFiles"
            @remove-upload="handleRemovePendingUpload"
          />

          <ChatScreen
            v-else
            ref="chatScreenRef"
            :messages="messages"
            :isGenerating="isGenerating"
            :currentStage="currentStage"
            :assistantResponse="assistantResponse"
            :assistantReasoning="assistantReasoning"
            :assistantSegments="assistantSegments"
            :assistantTokenUsage="assistantTokenUsage"
            :currentTurnToolEntries="currentTurnToolExecutionLogs"
            :pendingQuestion="pendingQuestion"
            :pendingPermissionRequestId="pendingPermissionRequestId"
            :agentMode="agentMode"
            :planMode="planMode"
            :pendingUploads="pendingUploads"
            :contextUsage="currentContextUsage"
            :contextCompacts="currentContextCompacts"
            :contextTokens="currentContextTokens"
            :compacting="isCompacting"
            @send="handleSendMessage"
            @save-user-edit="handleEditMessage($event)"
            @cancel="handleCancelGeneration"
            @mode-change="handleAgentModeChange"
            @upload-files="handleUploadFiles"
            @remove-upload="handleRemovePendingUpload"
            @ask-submit="handlePendingQuestionSubmit"
            @ask-skip="handlePendingQuestionSkip"
            @compact="handleCompactConversation"
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

      <WorkspaceDrawer
        v-if="mainView === 'chat'"
        :open="isDrawerOpen"
        :activeTab="activeWorkspaceTab"
        :entries="toolExecutionLogs"
        :currentTurnToolEntries="currentTurnToolExecutionLogs"
        :messages="messages"
        :files="conversationFiles"
        :assistantTurnCost="assistantTurnCost"
        :conversationId="activeConversationId || null"
        :browserOpenRequestKey="browserOpenRequestKey"
        :width="drawerWidth"
        @close="isDrawerOpen = false"
        @resize="handleDrawerResize"
        @resize-end="handleDrawerResizeEnd"
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
