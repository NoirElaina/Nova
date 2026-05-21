<script setup lang="ts">
import { ref, watch } from 'vue';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../lib/chat-types';
import type { RagDocumentMeta } from '../../features/chat/services/chat-api';
import CodeDiffTab from './workspace/CodeDiffTab.vue';
import FilesTab from './workspace/FilesTab.vue';
import UsageTab from './workspace/UsageTab.vue';
import BrowserTab from './workspace/BrowserTab.vue';

const emit = defineEmits<{
  (e: 'close'): void;
}>();

type TabId = 'diff' | 'usage' | 'files' | 'browser';

const props = defineProps<{
  open: boolean;
  activeTab?: TabId;
  selectedFileId?: string | null;
  entries: ToolExecutionEntry[];
  messages: ChatMessage[];
  files: RagDocumentMeta[];
  assistantTurnCost?: TurnCost;
  conversationId?: string | null;
}>();

const activeTab = ref<TabId>('diff');
const browserTabRef = ref<{ hideBrowserSurface?: () => void | Promise<void> } | null>(null);

const tabs: { id: TabId; label: string }[] = [
  { id: 'diff', label: '审查' },
  { id: 'usage', label: '用量' },
  { id: 'files', label: '文件' },
  { id: 'browser', label: '浏览器' },
];

watch(
  () => props.activeTab,
  (tab) => {
    if (tab) {
      activeTab.value = tab;
    }
  },
  { immediate: true },
);

const closeBrowserSurfaceBeforeLeave = () => {
  void browserTabRef.value?.hideBrowserSurface?.();
};
</script>

<template>
  <Transition name="slide-right" @before-leave="closeBrowserSurfaceBeforeLeave">
    <aside
      v-show="open"
      class="workspace-drawer-docked flex h-full flex-col"
    >
      <div class="flex h-full flex-col border-l border-[#e7e2d7] bg-[#faf9f6] shadow-2xl dark:border-[#333] dark:bg-[#1e1e1e]">
        <div class="flex h-14 shrink-0 items-center justify-between border-b border-[#e7e2d7] px-4 dark:border-[#333]">
          <div class="flex items-center gap-1">
            <button
              v-for="tab in tabs"
              :key="tab.id"
              :class="[
                'rounded-md px-3 py-1.5 text-sm font-medium transition-colors',
                activeTab === tab.id
                  ? 'bg-[#e8e3d8] text-[#1a1a1a] dark:bg-[#333] dark:text-[#ececec]'
                  : 'text-muted-foreground hover:bg-black/5 dark:hover:bg-white/5',
              ]"
              @click="activeTab = tab.id"
            >
              {{ tab.label }}
            </button>
          </div>

          <button
            class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-black/5 dark:hover:bg-white/5"
            @click="emit('close')"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="18" y1="6" x2="6" y2="18"/>
              <line x1="6" y1="6" x2="18" y2="18"/>
            </svg>
          </button>
        </div>

        <div class="min-h-0 flex-1 overflow-hidden">
          <CodeDiffTab
            v-if="activeTab === 'diff'"
          />

          <UsageTab
            v-else-if="activeTab === 'usage'"
            :entries="entries"
            :messages="messages"
            :assistantTurnCost="assistantTurnCost"
          />

          <FilesTab
            v-else-if="activeTab === 'files'"
            :files="files"
            :selectedFileId="selectedFileId"
          />

          <BrowserTab
            v-show="activeTab === 'browser'"
            ref="browserTabRef"
            :conversationId="conversationId"
            :visible="open && activeTab === 'browser'"
          />
        </div>
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
.workspace-drawer-docked {
  width: min(760px, 48vw);
  min-width: min(420px, 48vw);
  flex-shrink: 0;
  overflow: hidden;
}

.slide-right-enter-active,
.slide-right-leave-active {
  transition:
    width 0.28s cubic-bezier(0.22, 1, 0.36, 1),
    min-width 0.28s cubic-bezier(0.22, 1, 0.36, 1),
    transform 0.28s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.2s ease;
}

.slide-right-enter-from,
.slide-right-leave-to {
  width: 0;
  min-width: 0;
  transform: translateX(16px);
  opacity: 0;
}

.slide-right-enter-to,
.slide-right-leave-from {
  transform: translateX(0%);
  opacity: 1;
}

@media (max-width: 1100px) {
  .workspace-drawer-docked {
    width: 46vw;
    min-width: 360px;
  }
}
</style>
