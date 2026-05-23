<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import Sidebar from "./components/layout/Sidebar.vue";
import WelcomeScreen from "./components/chat/WelcomeScreen.vue";
import ChatScreen from "./components/chat/ChatScreen.vue";
import ExecutionTracePopover from "./components/chat/files/ExecutionTracePopover.vue";
import WorkspaceDrawer from "./components/chat/WorkspaceDrawer.vue";
import CustomScreen from "./components/custom/CustomScreen.vue";
import HooksConfigScreen from "./components/hooks/HooksConfigScreen.vue";
import AgentConfigScreen from "./components/agent/AgentConfigScreen.vue";
import AgentMarketScreen from "./components/agent/AgentMarketScreen.vue";
import ScheduleTaskScreen from "./components/schedule/ScheduleTaskScreen.vue";
import GlobalToastHost from "./components/layout/GlobalToastHost.vue";
import { useChatController } from "./features/chat/controllers/useChatController";
import {
  BROWSER_ANNOTATION_SELECTED_EVENT,
  type BrowserAnnotationSelectedPayload,
} from "./features/browser/browser-annotation";
import type { PendingUploadFile } from "./lib/chat-types";
import {
  exportConversation,
  exportRenderedConversationPdf,
  loadConversationHistory,
  type ConversationExportFormat,
} from "./features/chat/services/chat-api";
import { buildConversationExportHtml } from "./features/chat/utils/conversation-export-html";
import { emitToast } from "./lib/toast";

type WorkspaceTabId = "workspace" | "diff" | "usage" | "files" | "terminal" | "browser";
type BrowserOpenRequest = {
  conversationId?: string;
};

const {
  messages,
  isGenerating,
  currentStage,
  assistantResponse,
  assistantReasoning,
  assistantTokenUsage,
  assistantTurnCost,
  toolExecutionLogs,
  currentTurnToolExecutionLogs,
  conversations,
  activeConversationId,
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
} = useChatController();

void chatScreenRef;

const isDrawerOpen = ref(false);
const activeWorkspaceTab = ref<WorkspaceTabId>("workspace");
const browserOpenRequestKey = ref(0);
const exportingConversationId = ref<string | null>(null);
const exportingFormat = ref<ConversationExportFormat | null>(null);
let unlistenBrowserOpenRequest: UnlistenFn | null = null;
let unlistenBrowserAnnotationSelected: UnlistenFn | null = null;

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
});

onBeforeUnmount(() => {
  unlistenBrowserOpenRequest?.();
  unlistenBrowserAnnotationSelected?.();
});
</script>

<template>
  <div class="flex h-screen bg-[#fcfcfc] dark:bg-[#1a1a1a] text-[#1a1a1a] dark:text-[#ececec] overflow-hidden font-sans">
    <GlobalToastHost />
    
    <Sidebar
      v-if="isSidebarOpen"
      :recents="conversations"
      :activeConversationId="activeConversationId"
      :activeMainView="mainView"
      :exportingConversationId="exportingConversationId"
      :exportingFormat="exportingFormat"
      @new-chat="handleNewChat"
      @select-conversation="handleSelectConversation"
      @delete-conversation="handleDeleteConversation"
      @pin-conversation="handlePinConversation"
      @export-conversation="handleExportConversation"
      @change-main-view="handleChangeMainView"
      @toggle-sidebar="isSidebarOpen = !isSidebarOpen"
    />

    <!-- Main Content Area -->
    <main class="flex h-full min-w-0 flex-1 overflow-hidden">
      <section class="app-chat-pane relative flex h-full min-w-0 flex-1 flex-col">
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
          </div>

          <div v-if="mainView === 'chat'" class="flex items-center gap-2 pointer-events-auto">
            <ExecutionTracePopover :entries="toolExecutionLogs" />
            <Button
              variant="ghost"
              size="icon-sm"
              class="h-8 w-8 text-muted-foreground hover:bg-black/5 dark:hover:bg-white/5"
              :class="{ 'bg-black/5 dark:bg-white/10': isDrawerOpen }"
              title="工作区面板"
              @click="isDrawerOpen = !isDrawerOpen"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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

        <CustomScreen
          v-else-if="mainView === 'custom'"
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
            @send="handleSendMessage"
            @save-user-edit="handleEditMessage($event.index, $event.content)"
            @cancel="handleCancelGeneration"
            @mode-change="handleAgentModeChange"
            @upload-files="handleUploadFiles"
            @remove-upload="handleRemovePendingUpload"
            @ask-submit="handlePendingQuestionSubmit"
            @ask-skip="handlePendingQuestionSkip"
          />
        </template>
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
        @close="isDrawerOpen = false"
      />
    </main>
  </div>
</template>

<style>

html, body, #app {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
}

.app-chat-pane {
  transition: flex-basis 0.28s cubic-bezier(0.22, 1, 0.36, 1), width 0.28s cubic-bezier(0.22, 1, 0.36, 1);
}
</style>
