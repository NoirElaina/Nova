<script setup lang="ts">
import { computed } from 'vue';
import type { ConversationUsageSummary } from '@/lib/chat-types';
import ConversationUsageBar from '../ConversationUsageBar.vue';
import { subagentsFor } from '@/features/chat/services/subagents';
import { useLayoutStore } from '@/stores/layout';
import { Bot } from 'lucide-vue-next';

const props = defineProps<{
  activeAgent?: { id: string; name: string; description?: string } | null;
  conversationId?: string | null;
  conversationUsage?: ConversationUsageSummary | null;
}>();

const emit = defineEmits<{
  (e: 'remove-agent'): void;
}>();

const layoutStore = useLayoutStore();

// 子代理状态（行内按钮）：当前会话的子代理计数 + 协同工作区开关。
const subagentEntries = computed(() => subagentsFor(props.conversationId ?? null));
const subagentCount = computed(() => subagentEntries.value.length);
const subagentRunning = computed(
  () => subagentEntries.value.filter((entry) => entry.phase === 'running').length,
);

const isCollabOpen = computed(
  () => layoutStore.isDrawerOpen && layoutStore.activeWorkspaceTab === 'collab',
);

const handleClickSubagent = () => {
  layoutStore.toggleCollaborationTab();
};
</script>

<template>
  <!-- 输入框卡片外部下方状态行：左=当前智能体（无挂载时显示默认 Nova），右=会话用量 -->
  <div class="mt-1.5 flex items-start justify-between gap-3">
    <div class="min-w-0">
      <!-- 挂载了智能体：可点 × 卸载 -->
      <span
        v-if="activeAgent"
        class="inline-flex max-w-full items-center gap-1.5 rounded-md px-1.5 py-0.5 text-[12px] leading-tight text-[#4f5f73] transition-colors hover:bg-[#f1f5f9] dark:text-[#d5dbe3] dark:hover:bg-[#2a2a2a]"
        :title="activeAgent.description ? `智能体：${activeAgent.name} — ${activeAgent.description}` : `智能体：${activeAgent.name}`"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="11" width="18" height="10" rx="2" />
          <circle cx="12" cy="5" r="2" />
          <path d="M12 7v4" />
        </svg>
        <span class="truncate font-medium">{{ activeAgent.name }}</span>
        <button
          type="button"
          class="ml-0.5 flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center rounded-full opacity-60 transition-opacity hover:opacity-100"
          title="移除智能体，回到默认 Nova"
          aria-label="移除智能体"
          @click="emit('remove-agent')"
        >
          <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </span>
      <!-- 未挂载：显示默认 Nova 标签（仅标识，不可交互） -->
      <span
        v-else
        class="inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-[12px] leading-tight text-[#4f5f73] opacity-55 dark:text-[#d5dbe3]"
        title="当前对话使用默认智能体 Nova（全部能力）"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="11" width="18" height="10" rx="2" />
          <circle cx="12" cy="5" r="2" />
          <path d="M12 7v4" />
        </svg>
        <span class="font-medium">Nova（默认）</span>
      </span>
    </div>
    <div class="ml-auto min-w-0 shrink-0 flex items-center gap-2">
      <!-- 协同网络与子代理入口：当前会话有记录时显示，直接联动展开协同工作区面板 -->
      <button
        v-if="subagentCount > 0"
        type="button"
        class="inline-flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md px-1.5 py-0.5 text-[12px] leading-tight transition-colors hover:bg-muted"
        :class="isCollabOpen ? 'text-primary font-medium bg-muted' : 'text-muted-foreground'"
        :title="isCollabOpen ? '收起协同面板' : '查看协同与子代理执行详情'"
        @click="handleClickSubagent"
      >
        <Bot class="h-3.5 w-3.5" />
        <span class="font-medium">子代理</span>
        <span v-if="subagentRunning > 0" class="inline-flex items-center gap-1">
          <span class="inline-block h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
          <span class="tabular-nums text-emerald-600 dark:text-emerald-400">{{ subagentRunning }}</span>
        </span>
        <span v-else class="tabular-nums opacity-70">{{ subagentCount }}</span>
      </button>
      <ConversationUsageBar :usage="conversationUsage" />
    </div>
  </div>
</template>
