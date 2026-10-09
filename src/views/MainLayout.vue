<script setup lang="ts">
import { computed } from "vue";
import { useRouter, useRoute } from "vue-router";
import { Button } from "@/components/ui/button";
import Sidebar from "@/components/layout/Sidebar.vue";
import { useChatController } from "@/features/chat/controllers/useChatController";
import { useLayoutStore } from "@/stores/layout";
import { useConversationExport } from "@/features/chat/composables/useConversationExport";

const router = useRouter();
const route = useRoute();
const layoutStore = useLayoutStore();

const {
  conversations,
  activeConversationId,
  activeWorkspacePath,
  isSidebarOpen,
  handleNewChat,
  handleSelectConversation,
  handleDeleteConversation,
  handlePinConversation,
} = useChatController();

const {
  exportingConversationId,
  exportingFormat,
  handleExportConversation,
} = useConversationExport(conversations);

const currentMainView = computed(() => (route.name as any) || "chat");

const activeWorkspaceName = computed(() => {
  const path = activeWorkspacePath.value?.trim();
  if (!path) return "";
  const parts = path.replace(/\\/g, "/").split("/");
  const last = parts[parts.length - 1] || "";
  if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(last)) {
    return "默认工作区";
  }
  return last;
});

async function onNewChat() {
  await handleNewChat();
  if (route.name !== "chat" || route.params.conversationId) {
    await router.push("/chat");
  }
}

async function onSelectConversation(id: string) {
  await handleSelectConversation(id);
  if (route.name !== "chat" || route.params.conversationId !== id) {
    await router.push(`/chat/${id}`);
  }
}

async function onDeleteConversation(id: string) {
  await handleDeleteConversation(id);
  if (!activeConversationId.value && route.name === "chat") {
    await router.push("/chat");
  }
}

async function onChangeMainView(view: string) {
  if (view === "settings") {
    await router.push("/settings");
  } else if (view === "chat") {
    const target = activeConversationId.value ? `/chat/${activeConversationId.value}` : "/chat";
    await router.push(target);
  } else {
    await router.push(`/${view}`);
  }
}
</script>

<template>
  <div class="flex h-full w-full overflow-hidden">
    <!-- 侧边栏：由 isSidebarOpen 响应式控制显示/隐藏 -->
    <Sidebar
      v-if="isSidebarOpen"
      :recents="conversations"
      :activeConversationId="activeConversationId"
      :activeMainView="currentMainView"
      :exportingConversationId="exportingConversationId"
      :exportingFormat="exportingFormat"
      :width="layoutStore.sidebarWidth"
      @new-chat="onNewChat"
      @select-conversation="onSelectConversation"
      @delete-conversation="onDeleteConversation"
      @pin-conversation="handlePinConversation"
      @export-conversation="handleExportConversation"
      @change-main-view="onChangeMainView"
      @toggle-sidebar="isSidebarOpen = !isSidebarOpen"
      @resize="layoutStore.handleSidebarResize"
      @resize-end="layoutStore.handleSidebarResizeEnd"
    />

    <!-- 主工作区内容 -->
    <main class="relative flex h-full min-w-0 flex-1 overflow-hidden">
      <!-- 统一的顶部控制条 -->
      <header class="h-14 flex items-center justify-between px-4 absolute top-0 w-full z-10 pointer-events-none">
        <div class="flex items-center gap-2 pointer-events-auto">
          <Button
            variant="ghost"
            size="icon-sm"
            class="h-8 w-8 text-muted-foreground hover:bg-black/5 dark:hover:bg-white/5"
            title="切换侧边栏"
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

        <div v-if="currentMainView === 'chat'" class="flex items-center gap-2 pointer-events-auto">
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

      <!-- 嵌套路由插槽（Chat / Agent / Schedule / Hooks） -->
      <RouterView />
    </main>
  </div>
</template>
