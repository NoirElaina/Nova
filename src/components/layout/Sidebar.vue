<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface ConversationItem {
  id: string;
  title: string;
  pinnedAt?: number | null;
}

type MainView = "chat" | "custom" | "hooks" | "agent" | "agentMarket" | "schedule" | "settings";
type ConversationExportFormat = "json" | "pdf";

const props = defineProps<{
  recents: ConversationItem[];
  activeConversationId: string;
  activeMainView?: MainView;
  exportingConversationId: string | null;
  exportingFormat: ConversationExportFormat | null;
}>();

const emit = defineEmits<{
  (e: "toggle-sidebar"): void;
  (e: "new-chat"): void;
  (e: "select-conversation", id: string): void;
  (e: "delete-conversation", id: string): void;
  (e: "pin-conversation", id: string, pinned: boolean): void;
  (e: "export-conversation", id: string, format: ConversationExportFormat): void;
  (e: "change-main-view", view: MainView): void;
}>();

const sidebarRef = ref<HTMLElement | null>(null);
const openActionMenuId = ref<string | null>(null);
const exportDialogConversationId = ref<string | null>(null);

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

const closeRecentActionMenu = () => {
  openActionMenuId.value = null;
};

const toggleRecentActionMenu = (id: string) => {
  openActionMenuId.value = openActionMenuId.value === id ? null : id;
};

const isPinnedConversation = (item: ConversationItem) => Boolean(item.pinnedAt);

const exportDialogConversation = computed(() =>
  props.recents.find((item) => item.id === exportDialogConversationId.value) ?? null,
);

const isExportingDialogConversation = computed(() =>
  Boolean(
    exportDialogConversationId.value &&
      props.exportingConversationId === exportDialogConversationId.value,
  ),
);

const activeExportFormat = computed(() =>
  isExportingDialogConversation.value ? props.exportingFormat : null,
);

const sidebarItemClass =
  "h-8 w-full justify-start gap-2.5 rounded-md px-2.5 text-left text-[13px] font-normal transition-colors";
const sidebarItemActiveClass =
  "bg-white text-[#111827] shadow-[0_1px_1px_rgba(15,23,42,0.04)] ring-1 ring-[#e5e7eb] dark:bg-[#2b2b2b] dark:text-[#f5f5f5] dark:ring-[#383838]";
const sidebarItemIdleClass =
  "text-[#475569] hover:bg-white/70 hover:text-[#111827] dark:text-[#c8c8c8] dark:hover:bg-[#2a2a2a] dark:hover:text-[#f5f5f5]";
const sidebarSectionClass =
  "px-2.5 pb-1 pt-3 text-[11px] font-medium uppercase tracking-[0.04em] text-[#8a94a3] dark:text-[#858585]";

const handlePinConversation = (item: ConversationItem) => {
  emit("pin-conversation", item.id, !isPinnedConversation(item));
  closeRecentActionMenu();
};

const openExportDialog = (id: string) => {
  exportDialogConversationId.value = id;
  closeRecentActionMenu();
};

const closeExportDialog = () => {
  if (isExportingDialogConversation.value) {
    return;
  }
  exportDialogConversationId.value = null;
};

const handleExportConversation = (id: string, format: ConversationExportFormat) => {
  if (isExportingDialogConversation.value) {
    return;
  }
  emit("export-conversation", id, format);
};

const handleDeleteConversation = (id: string) => {
  emit("delete-conversation", id);
  closeRecentActionMenu();
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

  if (event.key === "Escape" && exportDialogConversationId.value) {
    event.preventDefault();
    closeExportDialog();
    return;
  }

  if (event.key === "Escape" && openActionMenuId.value) {
    event.preventDefault();
    closeRecentActionMenu();
    return;
  }

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

const onDocumentMouseDown = (event: MouseEvent) => {
  if (!openActionMenuId.value) {
    return;
  }

  const target = event.target as Node | null;
  if (target && sidebarRef.value?.contains(target)) {
    return;
  }

  closeRecentActionMenu();
};

onMounted(() => {
  window.addEventListener("keydown", onGlobalKeyDown);
  document.addEventListener("mousedown", onDocumentMouseDown);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onGlobalKeyDown);
  document.removeEventListener("mousedown", onDocumentMouseDown);
});

watch(
  () => props.exportingConversationId,
  (current, previous) => {
    if (!current && previous && exportDialogConversationId.value === previous) {
      exportDialogConversationId.value = null;
    }
  },
);
</script>

<template>
  <aside ref="sidebarRef" class="w-[225px] shrink-0 flex flex-col bg-[#f4f7fb] dark:bg-[#1f1f1f] border-r border-[#dfe6ee] dark:border-[#333] transition-all duration-300">
    <div class="flex flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-2 custom-scrollbar">
      <!-- Top Actions -->
      <Button variant="ghost" :class="[sidebarItemClass, sidebarItemIdleClass]" @click="emit('new-chat')">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" class="text-[#64748b]"><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>新对话</span>
      </Button>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, isSearchOpen ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="toggleSearch"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" class="text-[#64748b]"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="1.8"/><path d="M21 21l-4.35-4.35" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
        <span>搜索</span>
      </Button>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, props.activeMainView === 'custom' ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="emit('change-main-view', 'custom')"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" class="text-[#64748b]"><path d="M12 20.5V20m0-16v-.5m0 0a2.5 2.5 0 100 5 2.5 2.5 0 000-5zm0 16a2.5 2.5 0 100-5 2.5 2.5 0 000 5zm-8.5-8H4m16 0h-.5m0 0a2.5 2.5 0 10-5 0 2.5 2.5 0 005 0zm-16 0a2.5 2.5 0 105 0 2.5 2.5 0 00-5 0z" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>宠物</span>
      </Button>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, props.activeMainView === 'agentMarket' ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="emit('change-main-view', 'agentMarket')"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" class="text-[#64748b]">
          <path d="M4 7h16l-1 13H5L4 7Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>
          <path d="M8 7a4 4 0 0 1 8 0" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
          <path d="M9 12h6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
        </svg>
        <span>Agent 市场</span>
      </Button>

      <div v-if="isSearchOpen" class="px-0.5 pb-1 pt-1">
        <div class="relative">
          <Input
            ref="searchInputRef"
            v-model="searchKeyword"
            class="h-8 w-full rounded-lg border border-[#dbe3ea] bg-white px-8 pr-7 text-[13px] text-[#111827] outline-none focus:border-[#94a3b8] dark:border-[#3a3a3a] dark:bg-[#272727] dark:text-[#ececec] dark:focus:border-[#666]"
            placeholder="搜索会话标题（Enter 打开首条）"
            @keydown="onSearchKeyDown"
          />
          <svg class="absolute left-2.5 top-1/2 -translate-y-1/2 text-[#94a3b8]" width="14" height="14" viewBox="0 0 24 24" fill="none"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="1.8"/><path d="M21 21l-4.35-4.35" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
          <Button
            variant="ghost"
            size="icon-sm"
            v-if="searchKeyword"
            class="absolute right-1.5 top-1/2 h-5 w-5 -translate-y-1/2 rounded-md text-[#94a3b8] hover:bg-[#eef2f7] hover:text-[#334155] dark:hover:bg-[#3a3a3a]"
            @click="clearSearch"
            title="清除搜索"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M18 6 6 18M6 6l12 12" stroke-linecap="round"/></svg>
          </Button>
        </div>
        <div class="mt-1 px-1 text-[11px] text-[#8a94a3] dark:text-[#858585]">
          <span v-if="hasActiveSearch">匹配 {{ filteredRecents.length }} 条</span>
          <span v-else>快捷键：Ctrl/Cmd + K</span>
        </div>
      </div>

      <!-- 导航（已根据图片调整为中文标签与顺序） -->
      <h3 :class="sidebarSectionClass">导航</h3>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, props.activeMainView === 'chat' ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="emit('change-main-view', 'chat')"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>聊天</span>
      </Button>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, props.activeMainView === 'agent' ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="emit('change-main-view', 'agent')"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/><path d="M14 2v6h6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>智能体</span>
      </Button>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, props.activeMainView === 'schedule' ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="emit('change-main-view', 'schedule')"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.8"/><path d="M12 7v6l4 2" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>定时任务</span>
      </Button>
      <Button
        variant="ghost"
        :class="[sidebarItemClass, props.activeMainView === 'hooks' ? sidebarItemActiveClass : sidebarItemIdleClass]"
        @click="emit('change-main-view', 'hooks')"
      >
        <!-- 挂钩（复用搜索图标样式） -->
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none"><circle cx="11" cy="11" r="8" stroke="currentColor" stroke-width="1.8"/><path d="M21 21l-4.35-4.35" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>
        <span>挂钩</span>
      </Button>

      <!-- Recents -->
      <h3 :class="sidebarSectionClass">Recents</h3>
      <div
        v-for="recent in filteredRecents" 
        :key="recent.id"
        role="button"
        tabindex="0"
        class="group relative flex h-8 w-full cursor-pointer select-none items-center gap-2 rounded-md px-2.5 text-left text-[13px] transition-colors"
        :class="recent.id === props.activeConversationId
          ? sidebarItemActiveClass
          : 'text-[#334155] hover:bg-white/70 hover:text-[#111827] dark:text-[#ccc] dark:hover:bg-[#2a2a2a]'"
        @click="emit('select-conversation', recent.id)"
        @keydown.enter="emit('select-conversation', recent.id)"
        @keydown.space.prevent="emit('select-conversation', recent.id)"
      >
        <span class="truncate block flex-1">{{ recent.title || "New chat" }}</span>
        <span
          v-if="isPinnedConversation(recent)"
          class="h-1.5 w-1.5 flex-shrink-0 rounded-full bg-[#c87b57]"
          aria-label="已置顶"
          title="已置顶"
        />
        <Button
          variant="ghost"
          size="icon-sm"
          class="h-6 w-6 rounded-md p-1 text-[#94a3b8] transition-all hover:bg-[#eef2f7] hover:text-[#334155] dark:hover:bg-[#3a3a3a]"
          :class="openActionMenuId === recent.id ? 'opacity-100 bg-white shadow-sm dark:bg-[#3a3a3a]' : 'opacity-0 group-hover:opacity-100 focus:opacity-100'"
          title="会话操作"
          @click.stop="toggleRecentActionMenu(recent.id)"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <circle cx="5" cy="12" r="1.8"/>
            <circle cx="12" cy="12" r="1.8"/>
            <circle cx="19" cy="12" r="1.8"/>
          </svg>
        </Button>
        <div
          v-if="openActionMenuId === recent.id"
          class="absolute right-1 top-8 z-40 w-40 rounded-xl border border-[#e5e7eb] bg-white p-1 text-[13px] shadow-[0_12px_28px_rgba(15,23,42,0.12)] dark:border-[#3b3b3b] dark:bg-[#252525]"
          @click.stop
        >
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[#334155] hover:bg-[#f3f6fa] dark:text-[#ececec] dark:hover:bg-[#333]"
            @click.stop="handlePinConversation(recent)"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 17v5"/>
              <path d="M5 17h14"/>
              <path d="m7 9 5-5 5 5"/>
              <path d="M12 4v13"/>
            </svg>
            <span>{{ isPinnedConversation(recent) ? "取消置顶" : "置顶" }}</span>
          </button>
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[#334155] hover:bg-[#f3f6fa] dark:text-[#ececec] dark:hover:bg-[#333]"
            @click.stop="openExportDialog(recent.id)"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
              <path d="M7 10l5 5 5-5"/>
              <path d="M12 15V3"/>
            </svg>
            <span class="flex-1">导出</span>
            <span class="text-[11px] text-[#94a3b8]">选择</span>
          </button>
          <div class="my-1 h-px bg-[#e5e7eb] dark:bg-[#3a3a3a]" />
          <button
            type="button"
            class="flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[#dc2626] hover:bg-[#fef2f2] dark:hover:bg-[#3a2924]"
            @click.stop="handleDeleteConversation(recent.id)"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M3 6h18"/>
              <path d="M8 6V4h8v2"/>
              <path d="M19 6l-1 14H6L5 6"/>
              <path d="M10 11v6M14 11v6"/>
            </svg>
            <span>删除</span>
          </button>
        </div>
      </div>
      <div v-if="filteredRecents.length === 0" class="px-2.5 py-1.5 text-[13px] text-[#8a94a3]">
        {{ hasActiveSearch ? '未找到匹配会话' : '暂无历史会话' }}
      </div>

    </div>

    <!-- User Profile -->
    <div @click="emit('change-main-view', 'settings')" class="flex cursor-pointer items-center justify-between border-t border-[#dfe6ee] px-2.5 py-2 transition-colors hover:bg-white/70 dark:border-[#333] dark:hover:bg-[#2d2d2d]">
      <div class="flex items-center gap-2">
        <div class="flex h-7 w-7 items-center justify-center rounded-full bg-[#2f343b] text-[13px] font-medium text-white">N</div>
        <div class="flex flex-col">
          <span class="text-[13px] font-medium leading-tight text-[#111827] dark:text-[#ececec]">Nova</span>
        </div>
      </div>
      <div class="flex items-center gap-1">
        <Button variant="ghost" size="icon-sm" class="h-7 w-7 rounded-md text-[#64748b] hover:bg-white dark:hover:bg-[#444]">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="h-7 w-7 rounded-md text-[#64748b] hover:bg-white dark:hover:bg-[#444]">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none"><path d="M8 9l4-4 4 4M16 15l-4 4-4-4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </Button>
      </div>
    </div>
    <Teleport to="body">
      <Transition name="export-backdrop">
        <div
          v-if="exportDialogConversation"
          class="fixed inset-0 z-[95] flex items-center justify-center bg-[rgba(28,22,14,0.34)] px-5 backdrop-blur-[3px]"
          @click.self="closeExportDialog"
        >
          <Transition name="export-card">
            <div
              v-if="exportDialogConversation"
              class="w-full max-w-[430px] rounded-[24px] border border-[#e3d9ca] bg-[#fffdf8] p-5 shadow-[0_24px_80px_rgba(42,32,19,0.18)] dark:border-[#3c3933] dark:bg-[#272522]"
            >
              <div class="flex items-start justify-between gap-4">
                <div class="min-w-0">
                  <div class="text-[18px] font-semibold text-[#26221b] dark:text-[#f4efe7]">导出会话</div>
                  <div class="mt-1 truncate text-[13px] text-[#8a8072] dark:text-[#aaa197]">
                    {{ exportDialogConversation.title || "New chat" }}
                  </div>
                </div>
                <button
                  type="button"
                  class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-[#8c8172] transition-colors hover:bg-[#f0e9df] hover:text-[#2f2921] dark:hover:bg-[#393631] dark:hover:text-white"
                  aria-label="关闭导出选择"
                  @click="closeExportDialog"
                >
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M6 6l12 12M18 6 6 18" stroke-linecap="round"/>
                  </svg>
                </button>
              </div>

              <div class="mt-5 grid gap-3">
                <button
                  type="button"
                  class="group flex w-full items-center gap-3 rounded-2xl border border-[#e6ddcf] bg-[#faf6ee] px-4 py-3 text-left transition-all hover:-translate-y-0.5 hover:border-[#cdbca5] hover:bg-[#fffaf2] hover:shadow-[0_12px_30px_rgba(92,70,42,0.12)] disabled:cursor-wait disabled:opacity-75 disabled:hover:translate-y-0 disabled:hover:shadow-none dark:border-[#454037] dark:bg-[#302d28] dark:hover:border-[#6a5e50] dark:hover:bg-[#36322c]"
                  :disabled="isExportingDialogConversation"
                  @click="handleExportConversation(exportDialogConversation.id, 'json')"
                >
                  <span class="flex h-10 w-10 items-center justify-center rounded-xl bg-white text-[#8a6243] shadow-sm dark:bg-[#24221f] dark:text-[#e0c0a0]">
                    <span
                      v-if="activeExportFormat === 'json'"
                      class="h-4 w-4 animate-spin rounded-full border-2 border-[#c8b99f] border-t-[#8a6243]"
                    />
                    <svg v-else width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                      <path d="M14 2v6h6"/>
                      <path d="M8 13h8M8 17h5"/>
                    </svg>
                  </span>
                  <span class="min-w-0 flex-1">
                    <span class="block text-[15px] font-semibold text-[#2d271f] dark:text-[#f3eee7]">JSON 原始数据</span>
                    <span class="mt-0.5 block text-[12px] text-[#8c8172] dark:text-[#aaa197]">
                      {{ activeExportFormat === 'json' ? '正在写出会话数据...' : '适合备份、排查和后续导入处理' }}
                    </span>
                  </span>
                </button>

                <button
                  type="button"
                  class="group flex w-full items-center gap-3 rounded-2xl border border-[#e6ddcf] bg-[#faf6ee] px-4 py-3 text-left transition-all hover:-translate-y-0.5 hover:border-[#cdbca5] hover:bg-[#fffaf2] hover:shadow-[0_12px_30px_rgba(92,70,42,0.12)] disabled:cursor-wait disabled:opacity-75 disabled:hover:translate-y-0 disabled:hover:shadow-none dark:border-[#454037] dark:bg-[#302d28] dark:hover:border-[#6a5e50] dark:hover:bg-[#36322c]"
                  :disabled="isExportingDialogConversation"
                  @click="handleExportConversation(exportDialogConversation.id, 'pdf')"
                >
                  <span class="flex h-10 w-10 items-center justify-center rounded-xl bg-white text-[#b55941] shadow-sm dark:bg-[#24221f] dark:text-[#f0a38e]">
                    <span
                      v-if="activeExportFormat === 'pdf'"
                      class="h-4 w-4 animate-spin rounded-full border-2 border-[#e3b8aa] border-t-[#b55941]"
                    />
                    <svg v-else width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/>
                      <path d="M14 2v5h5"/>
                      <path d="M8 16h8M8 12h8"/>
                    </svg>
                  </span>
                  <span class="min-w-0 flex-1">
                    <span class="block text-[15px] font-semibold text-[#2d271f] dark:text-[#f3eee7]">PDF 阅读版</span>
                    <span class="mt-0.5 block text-[12px] text-[#8c8172] dark:text-[#aaa197]">
                      {{ activeExportFormat === 'pdf' ? '正在渲染并生成 PDF...' : '使用当前 Markdown 渲染效果生成' }}
                    </span>
                  </span>
                </button>
              </div>
            </div>
          </Transition>
        </div>
      </Transition>
    </Teleport>
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

.export-backdrop-enter-active,
.export-backdrop-leave-active {
  transition: opacity 0.18s ease;
}

.export-backdrop-enter-from,
.export-backdrop-leave-to {
  opacity: 0;
}

.export-card-enter-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.export-card-leave-active {
  transition: opacity 0.14s ease, transform 0.14s ease;
}

.export-card-enter-from,
.export-card-leave-to {
  opacity: 0;
  transform: translateY(8px) scale(0.98);
}
</style>
