<script setup lang="ts">
import { ref } from 'vue';
import { Button } from "@/components/ui/button";
import Sidebar from "./components/layout/Sidebar.vue";
import WelcomeScreen from "./components/chat/WelcomeScreen.vue";
import ChatScreen from "./components/chat/ChatScreen.vue";
import SessionFilesPopover from "./components/chat/files/SessionFilesPopover.vue";
import ExecutionTracePopover from "./components/chat/files/ExecutionTracePopover.vue";
import ShellSessionPopover from "./components/chat/files/ShellSessionPopover.vue";
import WorkspaceDrawer from "./components/chat/WorkspaceDrawer.vue";
import HooksConfigScreen from "./components/hooks/HooksConfigScreen.vue";
import AgentConfigScreen from "./components/agent/AgentConfigScreen.vue";
import ScheduleTaskScreen from "./components/schedule/ScheduleTaskScreen.vue";
import GlobalToastHost from "./components/layout/GlobalToastHost.vue";
import { useChatController } from "./features/chat/controllers/useChatController";
import {
  exportConversation,
  exportRenderedConversationPdf,
  loadConversationHistory,
  type ConversationExportFormat,
} from "./features/chat/services/chat-api";
import { buildConversationExportHtml } from "./features/chat/utils/conversation-export-html";
import { emitToast } from "./lib/toast";

type WorkspaceTabId = "diff" | "usage" | "files";

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
  refreshActiveConversationFiles,
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
const activeWorkspaceTab = ref<WorkspaceTabId>("diff");
const activeWorkspaceFileId = ref<string | null>(null);
const exportingConversationId = ref<string | null>(null);
const exportingFormat = ref<ConversationExportFormat | null>(null);

const openWorkspaceFile = (fileId: string) => {
  activeWorkspaceTab.value = "files";
  activeWorkspaceFileId.value = fileId;
  isDrawerOpen.value = true;
  void refreshActiveConversationFiles();
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
    <main class="flex-1 flex flex-col relative h-full">
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
          <SessionFilesPopover
            :files="conversationFiles"
            :pendingUploads="pendingUploads"
            @open="refreshActiveConversationFiles"
            @open-workspace-file="openWorkspaceFile"
            @remove-pending-upload="handleRemovePendingUpload"
          />
          <ShellSessionPopover
            :conversationId="activeConversationId || null"
            :refreshKey="toolExecutionLogs.length"
            :currentTurnToolEntries="currentTurnToolExecutionLogs"
          />
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

      <AgentConfigScreen
        v-else-if="mainView === 'agent'"
        @change-main-view="handleChangeMainView"
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

      <WorkspaceDrawer
        v-if="mainView === 'chat'"
          :open="isDrawerOpen"
          :activeTab="activeWorkspaceTab"
          :selectedFileId="activeWorkspaceFileId"
          :entries="toolExecutionLogs"
        :messages="messages"
        :files="conversationFiles"
        :assistantTurnCost="assistantTurnCost"
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
</style>
