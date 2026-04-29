<script setup lang="ts">
import { ref } from 'vue';
import type { FlowNodeEntry, ToolExecutionEntry } from '../../lib/chat-types';
import AgentFlowGraph from './AgentFlowGraph.vue';

defineProps<{
  open: boolean;
  entries: ToolExecutionEntry[];
  flowNodes: FlowNodeEntry[];
  isGenerating: boolean;
  hasMessages: boolean;
  lastUserMessage?: string;
  lastAssistantMessage?: string;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
}>();

type TabId = 'agent-flow' | 'diff';

const activeTab = ref<TabId>('agent-flow');

const tabs: { id: TabId; label: string }[] = [
  { id: 'agent-flow', label: 'Agent 流图' },
  { id: 'diff', label: 'Code Diff' },
];
</script>

<template>
  <Transition name="fade">
    <div
      v-if="open"
      class="absolute inset-0 z-20 bg-black/20 dark:bg-black/40"
      @click="emit('close')"
    />
  </Transition>

  <Transition name="slide-right">
    <div
      v-if="open"
      class="absolute top-0 right-0 z-30 flex h-full flex-col"
      style="width: 90%"
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
          <AgentFlowGraph
            v-if="activeTab === 'agent-flow'"
            :entries="entries"
            :flowNodes="flowNodes"
            :isGenerating="isGenerating"
            :hasMessages="hasMessages"
            :lastUserMessage="lastUserMessage"
            :lastAssistantMessage="lastAssistantMessage"
            class="h-full w-full"
          />

          <div
            v-else-if="activeTab === 'diff'"
            class="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground"
          >
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="opacity-40">
              <polyline points="16 18 22 12 16 6"/>
              <polyline points="8 6 2 12 8 18"/>
            </svg>
            <p class="text-sm">Code Diff 将在此显示</p>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.25s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.slide-right-enter-active,
.slide-right-leave-active {
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.slide-right-enter-from,
.slide-right-leave-to {
  transform: translateX(100%);
}

.slide-right-enter-to,
.slide-right-leave-from {
  transform: translateX(0%);
}
</style>
