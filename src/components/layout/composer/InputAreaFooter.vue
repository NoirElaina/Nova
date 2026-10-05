<script setup lang="ts">
import { computed } from 'vue';
import type { ConversationUsageSummary } from '../../../lib/chat-types';
import ConversationUsageBar from '../ConversationUsageBar.vue';
import {
  panelOpen as subagentPanelOpen,
  subagentsFor,
  togglePanel as toggleSubagentPanel,
} from '../../../features/chat/services/subagents';

const props = defineProps<{
  activeAgent?: { id: string; name: string; description?: string } | null;
  conversationId?: string | null;
  conversationUsage?: ConversationUsageSummary | null;
}>();

const emit = defineEmits<{
  (e: 'remove-agent'): void;
}>();

// 子代理状态（行内按钮）：当前会话的子代理计数 + 抽屉开关。
const subagentEntries = computed(() => subagentsFor(props.conversationId ?? null));
const subagentCount = computed(() => subagentEntries.value.length);
const subagentRunning = computed(
  () => subagentEntries.value.filter((entry) => entry.phase === 'running').length,
);
const panelOpen = subagentPanelOpen;
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
      <!-- 子代理按钮：当前会话有记录时显示 -->
      <button
        v-if="subagentCount > 0"
        type="button"
        class="inline-flex shrink-0 cursor-pointer items-center gap-1.5 rounded-md px-1.5 py-0.5 text-[12px] leading-tight transition-colors hover:bg-[#f1f5f9] dark:hover:bg-[#2a2a2a]"
        :class="panelOpen ? 'text-[#1a7f37] dark:text-[#4ade80]' : 'text-[#4f5f73] dark:text-[#d5dbe3]'"
        :title="panelOpen ? '收起子代理面板' : '查看子代理运行详情'"
        @click="toggleSubagentPanel"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="11" cy="11" r="7" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <span class="font-medium">子代理</span>
        <span v-if="subagentRunning > 0" class="inline-flex items-center gap-1">
          <span class="inline-block h-1.5 w-1.5 rounded-full bg-[#22c55e] animate-pulse" />
          <span class="tabular-nums">{{ subagentRunning }}</span>
        </span>
        <span v-else class="tabular-nums opacity-70">{{ subagentCount }}</span>
      </button>
      <ConversationUsageBar :usage="conversationUsage" />
    </div>
  </div>
</template>
