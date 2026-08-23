<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import {
  FileText,
  FolderOpen,
  Globe,
  Route,
  SquareTerminal,
  X,
} from 'lucide-vue-next';
import type { Component } from 'vue';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../lib/chat-types';
import type { SessionFileMeta } from '../../features/chat/services/chat-api';
import CodeDiffTab from './workspace/CodeDiffTab.vue';
import FilesTab from './workspace/FilesTab.vue';
import BrowserTab from './workspace/BrowserTab.vue';
import TraceTab from './workspace/TraceTab.vue';

const TerminalTab = defineAsyncComponent(() => import('./workspace/TerminalTab.vue'));

const emit = defineEmits<{
  (e: 'close'): void;
  (e: 'resize', width: number): void;
  (e: 'resize-end'): void;
}>();

type TabId = 'files' | 'diff' | 'terminal' | 'browser' | 'trace';

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

const activeTab = ref<TabId>('files');

// 工作区目录树已并入"文件"页签的子视图；计划已拆为独立侧边面板（PlanPanel）。
const tabs: { id: TabId; label: string; icon: Component }[] = [
  { id: 'files', label: '文件', icon: FolderOpen },
  { id: 'diff', label: '审查', icon: FileText },
  { id: 'terminal', label: '终端', icon: SquareTerminal },
  { id: 'browser', label: '浏览器', icon: Globe },
  { id: 'trace', label: '轨迹', icon: Route },
];

const selectTab = (id: TabId) => {
  activeTab.value = id;
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
      class="workspace-drawer-docked relative z-20 shrink-0 flex h-full flex-col"
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
      <div class="flex h-full flex-col py-2 pl-1.5 pr-2">
        <!-- 悬浮圆角卡片：四周留白 + 大圆角 + 柔和阴影，替代贴边直角矩形 -->
        <div
          class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-[#e7e9ee] bg-white shadow-[0_8px_30px_rgba(15,23,42,0.08)] dark:border-[#343434] dark:bg-[#1e1e1e] dark:shadow-[0_8px_30px_rgba(0,0,0,0.4)]"
        >
          <div class="flex h-11 shrink-0 items-center justify-between gap-2 border-b border-[#eef0f3] px-2 dark:border-[#2c2c2c]">
            <!-- 页签条：全部页签直接平铺，图标+文字，点击即切换 -->
            <div class="custom-scrollbar flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto">
              <button
                v-for="tab in tabs"
                :key="tab.id"
                type="button"
                class="flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-[12px] transition-colors"
                :class="activeTab === tab.id
                  ? 'bg-[#eef2f7] font-medium text-[#111827] dark:bg-white/12 dark:text-[#ececec]'
                  : 'text-[#64748b] hover:bg-[#f5f6f8] hover:text-[#334155] dark:text-[#8a8a8a] dark:hover:bg-white/5 dark:hover:text-[#ccc]'"
                :title="tab.label"
                @click="selectTab(tab.id)"
              >
                <component :is="tab.icon" class="h-3.5 w-3.5 shrink-0" />
                <span>{{ tab.label }}</span>
              </button>
            </div>

            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-black/5 dark:hover:bg-white/5"
              title="关闭面板"
              @click="emit('close')"
            >
              <X class="h-4 w-4" />
            </Button>
          </div>

          <div class="min-h-0 flex-1 overflow-hidden">
          <FilesTab
            v-if="activeTab === 'files'"
            :files="files"
            :selectedFileId="selectedFileId"
            :conversationId="conversationId ?? null"
          />

          <CodeDiffTab
            v-else-if="activeTab === 'diff'"
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

          <TraceTab
            v-if="activeTab === 'trace'"
            :conversationId="conversationId ?? null"
          />
          </div>
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
