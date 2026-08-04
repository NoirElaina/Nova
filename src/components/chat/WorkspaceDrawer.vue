<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../lib/chat-types';
import type { SessionFileMeta } from '../../features/chat/services/chat-api';
import CodeDiffTab from './workspace/CodeDiffTab.vue';
import FilesTab from './workspace/FilesTab.vue';
import BrowserTab from './workspace/BrowserTab.vue';
import PlanTab from './workspace/PlanTab.vue';
import WorkspaceOverviewTab from './workspace/WorkspaceOverviewTab.vue';

const TerminalTab = defineAsyncComponent(() => import('./workspace/TerminalTab.vue'));

const emit = defineEmits<{
  (e: 'close'): void;
  (e: 'resize', width: number): void;
  (e: 'resize-end'): void;
}>();

type TabId = 'workspace' | 'plan' | 'diff' | 'usage' | 'files' | 'terminal' | 'browser';

const props = defineProps<{
  open: boolean;
  activeTab?: TabId;
  selectedFileId?: string | null;
  entries: ToolExecutionEntry[];
  currentTurnToolEntries?: ToolExecutionEntry[];
  messages: ChatMessage[];
  files: SessionFileMeta[];
  assistantTurnCost?: TurnCost;
  conversationId?: string | null;
  browserOpenRequestKey?: number;
  /** 面板宽度（停靠式布局，占布局空间，可拖拽调整）。 */
  width?: number;
}>();

const activeTab = ref<TabId>('workspace');

const tabs: { id: TabId; label: string }[] = [
  { id: 'workspace', label: '工作区' },
  { id: 'plan', label: '计划' },
  { id: 'diff', label: '审查' },
  { id: 'files', label: '文件' },
  { id: 'terminal', label: '终端' },
  { id: 'browser', label: '浏览器' },
];

const activeTabMeta = computed(() => tabs.find((tab) => tab.id === activeTab.value) ?? tabs[0]);

// 页签只展示当前一个；点击页签或 + 号弹出列表切换/添加其它页签。
const isTabMenuOpen = ref(false);
const toggleTabMenu = () => {
  isTabMenuOpen.value = !isTabMenuOpen.value;
};
const selectTab = (id: TabId) => {
  activeTab.value = id;
  isTabMenuOpen.value = false;
};

watch(
  () => props.activeTab,
  (tab) => {
    if (tab) {
      activeTab.value = tab;
    }
  },
  { immediate: true },
);

// 面板宽度拖拽：左缘手柄按下后按指针位移 emit，App 层持有宽度并持久化。
const isResizing = ref(false);
let resizeStartX = 0;
let resizeStartWidth = 0;

const drawerWidthStyle = computed(() => ({
  width: `${props.width ?? 720}px`,
}));

const onResizeMove = (event: PointerEvent) => {
  if (!isResizing.value) return;
  event.preventDefault();
  // 面板在右侧，指针向左移动（clientX 减小）面板变宽。
  emit('resize', resizeStartWidth + (resizeStartX - event.clientX));
};

const stopResize = () => {
  if (!isResizing.value) return;
  isResizing.value = false;
  document.body.style.cursor = '';
  document.body.style.userSelect = '';
  emit('resize-end');
};

const startResize = (event: PointerEvent) => {
  event.preventDefault();
  (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
  resizeStartX = event.clientX;
  resizeStartWidth = props.width ?? 720;
  isResizing.value = true;
  document.body.style.cursor = 'col-resize';
  document.body.style.userSelect = 'none';
};

onBeforeUnmount(() => {
  stopResize();
});

</script>

<template>
  <Transition name="slide-right">
    <aside
      v-show="open"
      class="workspace-drawer-docked relative shrink-0 flex h-full flex-col"
      :class="isResizing ? '' : 'transition-[width] duration-200'"
      :style="drawerWidthStyle"
    >
      <!-- 左缘拖拽手柄：调整面板宽度，命中区 12px 横跨边界 -->
      <div
        class="absolute -left-1.5 top-0 z-40 h-full w-3 cursor-col-resize"
        title="拖动调整面板宽度"
        @pointerdown="startResize"
        @pointermove="onResizeMove"
        @pointerup="stopResize"
        @pointercancel="stopResize"
      >
        <div
          class="absolute left-1/2 top-0 h-full w-[2px] -translate-x-1/2 transition-colors"
          :class="isResizing ? 'bg-[#94a3b8]/70' : 'bg-transparent hover:bg-[#94a3b8]/40'"
        />
      </div>
      <div class="flex h-full flex-col border-l border-[#e5e7eb] bg-white dark:border-[#333] dark:bg-[#1e1e1e]">
        <div class="relative flex h-10 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-2 dark:border-[#333]">
          <!-- 页签条：只显示当前页签（胶囊样式，参考应用同款），+ 号弹出列表切换/添加 -->
          <div class="flex min-w-0 items-center gap-1">
            <button
              type="button"
              class="flex h-7 max-w-[180px] items-center gap-1 rounded-md bg-[#f3f4f6] px-2.5 text-[13px] font-medium text-[#111827] transition-colors hover:bg-[#e9ebef] dark:bg-white/10 dark:text-[#ececec] dark:hover:bg-white/15"
              :title="activeTabMeta.label"
              @click="toggleTabMenu"
            >
              <span class="truncate">{{ activeTabMeta.label }}</span>
              <svg
                width="11"
                height="11"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.2"
                stroke-linecap="round"
                stroke-linejoin="round"
                class="shrink-0 text-[#64748b] transition-transform dark:text-[#9ca3af]"
                :class="isTabMenuOpen ? 'rotate-180' : ''"
              >
                <path d="M6 9l6 6 6-6"/>
              </svg>
            </button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="h-7 w-7 shrink-0 rounded-md text-[#64748b] hover:bg-[#f5f6f8] dark:text-muted-foreground dark:hover:bg-white/5"
              title="添加页签"
              @click="toggleTabMenu"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                <path d="M12 5v14M5 12h14"/>
              </svg>
            </Button>
          </div>

          <!-- 点击外部关闭菜单的透明遮罩 -->
          <div v-if="isTabMenuOpen" class="fixed inset-0 z-40" @click="isTabMenuOpen = false" />
          <!-- 页签列表弹层 -->
          <div
            v-if="isTabMenuOpen"
            class="absolute left-2 top-[42px] z-50 w-40 rounded-xl border border-[#e5e7eb] bg-white p-1 shadow-[0_12px_28px_rgba(15,23,42,0.12)] dark:border-[#3b3b3b] dark:bg-[#252525]"
          >
            <button
              v-for="tab in tabs"
              :key="tab.id"
              type="button"
              class="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-[13px] transition-colors"
              :class="activeTab === tab.id
                ? 'bg-[#f3f6fa] font-medium text-[#111827] dark:bg-white/10 dark:text-[#ececec]'
                : 'text-[#334155] hover:bg-[#f3f6fa] dark:text-[#ccc] dark:hover:bg-white/5'"
              @click="selectTab(tab.id)"
            >
              {{ tab.label }}
              <svg
                v-if="activeTab === tab.id"
                width="13"
                height="13"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.4"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <polyline points="20 6 9 17 4 12"/>
              </svg>
            </button>
          </div>

          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            class="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-black/5 dark:hover:bg-white/5"
            @click="emit('close')"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="18" y1="6" x2="6" y2="18"/>
              <line x1="6" y1="6" x2="18" y2="18"/>
            </svg>
          </Button>
        </div>

        <div class="min-h-0 flex-1 overflow-hidden">
          <WorkspaceOverviewTab
            v-if="activeTab === 'workspace'"
            :conversationId="conversationId ?? null"
          />

          <PlanTab
            v-else-if="activeTab === 'plan'"
            :conversationId="conversationId ?? null"
          />

          <CodeDiffTab
            v-else-if="activeTab === 'diff'"
            :conversationId="conversationId ?? null"
          />

          <FilesTab
            v-else-if="activeTab === 'files'"
            :files="files"
            :selectedFileId="selectedFileId"
            :conversationId="conversationId ?? null"
          />

          <TerminalTab
            v-show="activeTab === 'terminal'"
            :conversationId="conversationId ?? null"
            :visible="open && activeTab === 'terminal'"
            :entries="entries"
            :currentTurnToolEntries="currentTurnToolEntries"
          />

          <BrowserTab
            v-show="activeTab === 'browser'"
            :conversationId="conversationId"
            :visible="open && activeTab === 'browser'"
            :openRequestKey="browserOpenRequestKey"
          />
        </div>
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
/* 停靠式面板：与参考应用一致，占据布局空间压缩聊天区，宽度可拖拽调整 */
.workspace-drawer-docked {
  flex-shrink: 0;
  overflow: hidden;
}

.custom-scrollbar::-webkit-scrollbar {
  height: 4px;
}
.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}
.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: var(--color-border, #e5e5e5);
  border-radius: 10px;
}

.slide-right-enter-active,
.slide-right-leave-active {
  transition:
    width 0.24s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.2s ease;
}

.slide-right-enter-from,
.slide-right-leave-to {
  width: 0 !important;
  opacity: 0;
}

.slide-right-enter-to,
.slide-right-leave-from {
  opacity: 1;
}
</style>
