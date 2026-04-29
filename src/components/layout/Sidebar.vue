<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import SettingsModal from "./settings/SettingsModal.vue";

interface ConversationItem {
  id: string;
  title: string;
}

type MainView = "chat" | "hooks" | "agent" | "schedule";

const props = defineProps<{
  recents: ConversationItem[];
  activeConversationId: string;
  activeMainView?: MainView;
}>();

const emit = defineEmits<{
  (e: "toggle-sidebar"): void;
  (e: "new-chat"): void;
  (e: "select-conversation", id: string): void;
  (e: "delete-conversation", id: string): void;
  (e: "change-main-view", view: MainView): void;
}>();

const isSettingsOpen = ref(false);
const openSettings = () => {
  isSettingsOpen.value = true;
};

const isSearchOpen = ref(false);
const searchKeyword = ref("");
const searchInputRef = ref<unknown>(null);

const normalizedSearchKeyword = computed(() => searchKeyword.value.trim().toLocaleLowerCase());
const hasActiveSearch = computed(() => normalizedSearchKeyword.value.length > 0);

const filteredRecents = computed(() => {
  const keyword = normalizedSearchKeyword.value;
  if (!keyword) {
    return props.recents;
  }

  return props.recents.filter((item) =>
    (item.title || "New chat").toLocaleLowerCase().includes(keyword),
  );
});

const resolveSearchInputElement = (): HTMLInputElement | null => {
  const refValue = searchInputRef.value as {
    $el?: unknown;
    focus?: () => void;
    select?: () => void;
  } | null;

  if (!refValue) {
    return null;
  }

  if (refValue instanceof HTMLInputElement) {
    return refValue;
  }

  if (refValue.$el instanceof HTMLInputElement) {
    return refValue.$el;
  }

  return null;
};

const focusSearchInput = async () => {
  await nextTick();
  const input = resolveSearchInputElement();
  input?.focus();
  input?.select();
};

const openSearch = async () => {
  isSearchOpen.value = true;
  await focusSearchInput();
};

const closeSearch = () => {
  searchKeyword.value = "";
  isSearchOpen.value = false;
};

const toggleSearch = async () => {
  if (isSearchOpen.value) {
    closeSearch();
    return;
  }
  await openSearch();
};

const clearSearch = async () => {
  searchKeyword.value = "";
  await focusSearchInput();
};

const selectFirstSearchResult = () => {
  if (filteredRecents.value.length === 0) {
    return;
  }
  emit("select-conversation", filteredRecents.value[0].id);
};

const onSearchKeyDown = (event: KeyboardEvent) => {
  if (event.key === "Escape") {
    event.preventDefault();
    if (searchKeyword.value) {
      searchKeyword.value = "";
      return;
    }
    closeSearch();
    return;
  }

  if (event.key === "Enter") {
    event.preventDefault();
    selectFirstSearchResult();
  }
};

const onGlobalKeyDown = (event: KeyboardEvent) => {
  const key = event.key.toLocaleLowerCase();
  const target = event.target as HTMLElement | null;
  const tagName = target?.tagName.toLocaleLowerCase() ?? "";
  const isEditable =
    target?.isContentEditable ||
    tagName === "input" ||
    tagName === "textarea" ||
    tagName === "select";

  if ((event.ctrlKey || event.metaKey) && key === "k") {
    event.preventDefault();
    void openSearch();
    return;
  }

  if (!isEditable && !event.ctrlKey && !event.metaKey && !event.altKey && key === "/") {
    event.preventDefault();
    void openSearch();
  }
};

onMounted(() => {
  window.addEventListener("keydown", onGlobalKeyDown);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onGlobalKeyDown);
});
</script>

<template>
  <aside class="w-[260px] flex-shrink-0 flex flex-col bg-[#faecd/30] bg-[#f9f9f8] dark:bg-[#1f1f1f] border-r border-[#e5e5e5] dark:border-[#333] transition-all duration-300">
    <div class="p-3 flex flex-col gap-1 overflow-y-auto flex-1 custom-scrollbar">
      <!-- Top Actions -->
      <Button variant="ghost" class="w-full justify-start gap-3 px-3 py-2 text-left font-medium hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d]" @click="emit('new-chat')">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" class="text-muted-foreground"><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span class="text-[0.9rem]">新对话</span>
      </Button>
      <Button
        variant="ghost"
        class="w-full justify-start gap-3 px-3 py-2 text-left font-medium"
        :class="isSearchOpen ? 'bg-[#ebebeb] dark:bg-[#2d2d2d]' : 'hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d]'"
        @click="toggleSearch"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" class="text-muted-foreground"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="2"/><path d="M21 21l-4.35-4.35" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>
        <span class="text-[0.9rem]">搜索</span>
      </Button>
      <Button variant="ghost" class="mb-4 w-full justify-start gap-3 px-3 py-2 text-left font-medium hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d]" @click="openSettings">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" class="text-muted-foreground"><path d="M12 20.5V20m0-16v-.5m0 0a2.5 2.5 0 100 5 2.5 2.5 0 000-5zm0 16a2.5 2.5 0 100-5 2.5 2.5 0 000 5zm-8.5-8H4m16 0h-.5m0 0a2.5 2.5 0 10-5 0 2.5 2.5 0 005 0zm-16 0a2.5 2.5 0 105 0 2.5 2.5 0 00-5 0z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span class="text-[0.9rem]">自定义</span>
      </Button>

      <div v-if="isSearchOpen" class="px-2 pb-2">
        <div class="relative">
          <Input
            ref="searchInputRef"
            v-model="searchKeyword"
            class="w-full h-9 rounded-lg border border-[#dfdbd2] dark:border-[#3a3a3a] bg-white dark:bg-[#272727] px-9 pr-8 text-[0.85rem] text-[#2b2b2b] dark:text-[#ececec] outline-none focus:border-[#c4b49f] dark:focus:border-[#666]"
            placeholder="搜索会话标题（Enter 打开首条）"
            @keydown="onSearchKeyDown"
          />
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 text-[#a59f93]" width="14" height="14" viewBox="0 0 24 24" fill="none"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="2"/><path d="M21 21l-4.35-4.35" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>
          <Button
            variant="ghost"
            size="icon-sm"
            v-if="searchKeyword"
            class="absolute right-2 top-1/2 h-5 w-5 -translate-y-1/2 rounded-md text-[#9d968a] hover:bg-[#efebe3] hover:text-[#6e675b] dark:hover:bg-[#3a3a3a]"
            @click="clearSearch"
            title="清除搜索"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6 6 18M6 6l12 12" stroke-linecap="round"/></svg>
          </Button>
        </div>
        <div class="mt-1 px-1 text-[11px] text-[#9a9386] dark:text-[#8f8a80]">
          <span v-if="hasActiveSearch">匹配 {{ filteredRecents.length }} 条</span>
          <span v-else>快捷键：Ctrl/Cmd + K</span>
        </div>
      </div>

      <!-- 导航（已根据图片调整为中文标签与顺序） -->
      <h3 class="text-xs font-semibold text-[#8b8b8b] px-3 mt-2 mb-1">导航</h3>
      <Button
        variant="ghost"
        class="w-full justify-start gap-3 px-3 py-2 text-left"
        :class="props.activeMainView === 'chat'
          ? 'bg-[#ebebeb] dark:bg-[#2d2d2d] text-[#222] dark:text-[#f2f2f2]'
          : 'hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d] text-muted-foreground'"
        @click="emit('change-main-view', 'chat')"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span class="text-[0.9rem]">聊天</span>
      </Button>
      <Button
        variant="ghost"
        class="w-full justify-start gap-3 px-3 py-2 text-left"
        :class="props.activeMainView === 'agent'
          ? 'bg-[#ebebeb] dark:bg-[#2d2d2d] text-[#222] dark:text-[#f2f2f2]'
          : 'hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d] text-muted-foreground'"
        @click="emit('change-main-view', 'agent')"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 2v6h6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span class="text-[0.9rem]">智能体</span>
      </Button>
      <Button
        variant="ghost"
        class="w-full justify-start gap-3 px-3 py-2 text-left"
        :class="props.activeMainView === 'schedule'
          ? 'bg-[#ebebeb] dark:bg-[#2d2d2d] text-[#222] dark:text-[#f2f2f2]'
          : 'hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d] text-muted-foreground'"
        @click="emit('change-main-view', 'schedule')"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="2"/><path d="M12 7v6l4 2" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span class="text-[0.9rem]">定时任务</span>
      </Button>
      <Button
        variant="ghost"
        class="w-full justify-start gap-3 px-3 py-2 text-left"
        :class="props.activeMainView === 'hooks'
          ? 'bg-[#ebebeb] dark:bg-[#2d2d2d] text-[#222] dark:text-[#f2f2f2]'
          : 'hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d] text-muted-foreground'"
        @click="emit('change-main-view', 'hooks')"
      >
        <!-- 挂钩（复用搜索图标样式） -->
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="2"/><path d="M21 21l-4.35-4.35" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>
        <span class="text-[0.9rem]">挂钩</span>
      </Button>
      <div class="mb-4" />

      <!-- Recents -->
      <h3 class="text-xs font-semibold text-[#8b8b8b] px-3 mt-2 mb-1">Recents</h3>
      <div
        v-for="recent in filteredRecents" 
        :key="recent.id"
        role="button"
        tabindex="0"
        class="group flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left text-[0.85rem]"
        :class="recent.id === props.activeConversationId
          ? 'bg-[#ebebeb] dark:bg-[#2d2d2d] text-[#222] dark:text-[#f2f2f2]'
          : 'hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d] text-[#333] dark:text-[#ccc]'"
        @click="emit('select-conversation', recent.id)"
        @keydown.enter="emit('select-conversation', recent.id)"
        @keydown.space.prevent="emit('select-conversation', recent.id)"
      >
        <span class="truncate block flex-1">{{ recent.title || "New chat" }}</span>
        <Button
          variant="ghost"
          size="icon-sm"
          class="opacity-0 group-hover:opacity-100 transition-opacity text-[#9b9b9b] hover:text-[#da7756] p-1 rounded"
          title="Delete conversation"
          @click.stop="emit('delete-conversation', recent.id)"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 6h18"/>
            <path d="M8 6V4h8v2"/>
            <path d="M19 6l-1 14H6L5 6"/>
            <path d="M10 11v6M14 11v6"/>
          </svg>
        </Button>
      </div>
      <div v-if="filteredRecents.length === 0" class="px-3 py-1.5 text-[0.85rem] text-[#8b8b8b]">
        {{ hasActiveSearch ? '未找到匹配会话' : '暂无历史会话' }}
      </div>

    </div>

    <!-- User Profile -->
    <div @click="openSettings" class="p-3 border-t border-[#e5e5e5] dark:border-[#333] flex items-center justify-between hover:bg-[#ebebeb] dark:hover:bg-[#2d2d2d] transition-colors cursor-pointer rounded-b-xl">
      <div class="flex items-center gap-2">
        <div class="w-8 h-8 rounded-full bg-[#3d3d3d] text-white flex items-center justify-center font-medium text-sm">N</div>
        <div class="flex flex-col">
          <span class="text-sm font-medium leading-tight">Nova</span>
        </div>
      </div>
      <div class="flex items-center gap-1">
        <Button variant="ghost" size="icon-sm" class="h-7 w-7 text-muted-foreground hover:bg-[#d4d4d4] dark:hover:bg-[#444]">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="h-7 w-7 text-muted-foreground hover:bg-[#d4d4d4] dark:hover:bg-[#444]">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="M8 9l4-4 4 4M16 15l-4 4-4-4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </Button>
      </div>
    </div>
    <SettingsModal v-model="isSettingsOpen" />
  </aside>
</template>

<style scoped>
.custom-scrollbar::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}
.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: var(--color-border, #e5e5e5);
  border-radius: 10px;
}
.dark .custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: #444;
}
</style>
