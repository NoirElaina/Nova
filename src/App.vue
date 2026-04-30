<script setup lang="ts">
import { ref } from 'vue';
import { Button } from "@/components/ui/button";
import Sidebar from "./components/layout/Sidebar.vue";
import WelcomeScreen from "./components/chat/WelcomeScreen.vue";
import ChatScreen from "./components/chat/ChatScreen.vue";
import SessionFilesPopover from "./components/chat/files/SessionFilesPopover.vue";
import ExecutionTracePopover from "./components/chat/files/ExecutionTracePopover.vue";
import WorkspaceDrawer from "./components/chat/WorkspaceDrawer.vue";
import HooksConfigScreen from "./components/hooks/HooksConfigScreen.vue";
import AgentConfigScreen from "./components/agent/AgentConfigScreen.vue";
import ScheduleTaskScreen from "./components/schedule/ScheduleTaskScreen.vue";
import GlobalToastHost from "./components/layout/GlobalToastHost.vue";
import { useChatController } from "./features/chat/controllers/useChatController";

const {
  messages,
  isGenerating,
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
  pendingQuestion,
  pendingPermissionRequestId,
  agentMode,
  planMode,
  mainView,
  isSidebarOpen,
  chatScreenRef,
  refreshActiveConversationFiles,
  handleSendMessage,
  handleUploadFiles,
  handleRemovePendingUpload,
  handleCancelGeneration,
  handlePendingQuestionSubmit,
  handlePendingQuestionSkip,
  handleAgentModeChange,
  handleNewChat,
  handleSelectConversation,
  handleDeleteConversation,
  handleChangeMainView,
} = useChatController();

void chatScreenRef;

const isDrawerOpen = ref(false);
</script>

<template>
  <div class="flex h-screen bg-[#fcfcfc] dark:bg-[#1a1a1a] text-[#1a1a1a] dark:text-[#ececec] overflow-hidden font-sans">
    <GlobalToastHost />
    
    <Sidebar
      v-if="isSidebarOpen"
      :recents="conversations"
      :activeConversationId="activeConversationId"
      :activeMainView="mainView"
      @new-chat="handleNewChat"
      @select-conversation="handleSelectConversation"
      @delete-conversation="handleDeleteConversation"
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
          <div class="flex gap-1 ml-2 text-muted-foreground/40 hidden md:flex">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5M12 19l-7-7 7-7"/></svg>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M12 5l7 7-7 7"/></svg>
          </div>
        </div>

        <div v-if="mainView === 'chat'" class="flex items-center gap-2 pointer-events-auto">
          <SessionFilesPopover
            :files="conversationFiles"
            :pendingUploads="pendingUploads"
            @open="refreshActiveConversationFiles"
            @remove-pending-upload="handleRemovePendingUpload"
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
          :assistantResponse="assistantResponse"
          :assistantReasoning="assistantReasoning"
          :assistantTokenUsage="assistantTokenUsage"
          :currentTurnToolEntries="currentTurnToolExecutionLogs"
          :pendingQuestion="pendingQuestion"
          :pendingPermissionRequestId="pendingPermissionRequestId"
          :agentMode="agentMode"
          :planMode="planMode"
          :pendingUploads="pendingUploads"
          @send="handleSendMessage"
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
        :entries="toolExecutionLogs"
        :messages="messages"
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
