<script setup lang="ts">
import { defineAsyncComponent, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../lib/chat-types';
import type { SessionFileMeta } from '../../features/chat/services/chat-api';
import CodeDiffTab from './workspace/CodeDiffTab.vue';
import FilesTab from './workspace/FilesTab.vue';
import BrowserTab from './workspace/BrowserTab.vue';
import WorkspaceOverviewTab from './workspace/WorkspaceOverviewTab.vue';

const TerminalTab = defineAsyncComponent(() => import('./workspace/TerminalTab.vue'));

const emit = defineEmits<{
  (e: 'close'): void;
}>();

type TabId = 'workspace' | 'diff' | 'usage' | 'files' | 'terminal' | 'browser';

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
}>();

const activeTab = ref<TabId>('workspace');

const tabs: { id: TabId; label: string }[] = [
  { id: 'workspace', label: '工作区' },
  { id: 'diff', label: '审查' },
  { id: 'files', label: '文件' },
  { id: 'terminal', label: '终端' },
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

</script>

<template>
  <Transition name="slide-right">
    <aside
      v-show="open"
      class="workspace-drawer-docked flex h-full flex-col"
    >
      <div class="flex h-full flex-col border-l border-[#e5e7eb] bg-white dark:border-[#333] dark:bg-[#1e1e1e]">
        <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 dark:border-[#333]">
          <div class="flex items-center gap-0.5">
            <Button
              v-for="tab in tabs"
              :key="tab.id"
              type="button"
              variant="ghost"
              size="sm"
              :class="[
                'h-7 rounded-lg px-2.5 py-0 text-[13px] font-normal transition-colors',
                activeTab === tab.id
                  ? 'bg-[#f7f7f8] text-[#111827] dark:bg-[#2b2b2b] dark:text-[#ececec]'
                  : 'text-[#64748b] hover:bg-[#f8f9fa] dark:text-muted-foreground dark:hover:bg-white/5',
              ]"
              @click="activeTab = tab.id"
            >
              {{ tab.label }}
            </Button>
          </div>

          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            class="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-black/5 dark:hover:bg-white/5"
            @click="emit('close')"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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

          <CodeDiffTab
            v-else-if="activeTab === 'diff'"
            :conversationId="conversationId ?? null"
          />

          <FilesTab
            v-else-if="activeTab === 'files'"
            :files="files"
            :selectedFileId="selectedFileId"
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
