<script setup lang="ts">
import { computed } from 'vue';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../../lib/chat-types';

const props = defineProps<{
  entries: ToolExecutionEntry[];
  messages: ChatMessage[];
  assistantTurnCost?: TurnCost;
}>();

const assistantMessages = computed(() =>
  props.messages.filter((message) => message.role === 'assistant'),
);

const totalInputTokens = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.inputTokens ?? 0), 0),
);

const totalOutputTokens = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.outputTokens ?? 0), 0),
);

const totalTokens = computed(() => totalInputTokens.value + totalOutputTokens.value);

const totalToolCalls = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.toolCalls ?? 0), 0),
);

const totalToolDurationMs = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.toolDurationMs ?? 0), 0),
);

const latestAssistantCost = computed(() => {
  if (props.assistantTurnCost) {
    return props.assistantTurnCost;
  }

  for (let i = props.messages.length - 1; i >= 0; i -= 1) {
    const message = props.messages[i];
    if (message.role === 'assistant' && message.cost) {
      return message.cost;
    }
  }

  return null;
});

const completedTools = computed(() =>
  props.entries.filter((entry) => entry.status === 'completed').length,
);

const failedTools = computed(() =>
  props.entries.filter((entry) => entry.status === 'error').length,
);

const cancelledTools = computed(() =>
  props.entries.filter((entry) => entry.status === 'cancelled').length,
);

const formatNumber = (value: number) => value.toLocaleString('en-US');
const formatDuration = (value: number) =>
  value >= 1000 ? `${(value / 1000).toFixed(1)} s` : `${value} ms`;
</script>

<template>
  <div class="h-full overflow-y-auto px-4 py-4">
    <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <div class="usage-card">
        <div class="usage-label">总 Tokens</div>
        <div class="usage-value">{{ formatNumber(totalTokens) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">Prompt Tokens</div>
        <div class="usage-value">{{ formatNumber(totalInputTokens) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">输出 Tokens</div>
        <div class="usage-value">{{ formatNumber(totalOutputTokens) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">Tool Calls</div>
        <div class="usage-value">{{ formatNumber(totalToolCalls) }}</div>
      </div>
    </div>

    <div class="mt-4 grid grid-cols-2 gap-3 lg:grid-cols-4">
      <div class="usage-card">
        <div class="usage-label">Tool 耗时</div>
        <div class="usage-value">{{ formatDuration(totalToolDurationMs) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">已完成工具</div>
        <div class="usage-value">{{ formatNumber(completedTools) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">失败工具</div>
        <div class="usage-value">{{ formatNumber(failedTools) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">取消工具</div>
        <div class="usage-value">{{ formatNumber(cancelledTools) }}</div>
      </div>
    </div>

    <div class="mt-4 rounded-xl border border-[#e7e2d7] bg-white/80 p-4 dark:border-[#333] dark:bg-[#252525]">
      <div class="text-sm font-medium text-[#1a1a1a] dark:text-[#ececec]">最近一轮</div>
      <div
        v-if="latestAssistantCost"
        class="mt-3 grid grid-cols-2 gap-3 lg:grid-cols-4"
      >
        <div>
          <div class="usage-label">Prompt</div>
          <div class="usage-detail">{{ formatNumber(latestAssistantCost.inputTokens) }}</div>
        </div>
        <div>
          <div class="usage-label">输出</div>
          <div class="usage-detail">{{ formatNumber(latestAssistantCost.outputTokens) }}</div>
        </div>
        <div>
          <div class="usage-label">工具调用</div>
          <div class="usage-detail">{{ formatNumber(latestAssistantCost.toolCalls) }}</div>
        </div>
        <div>
          <div class="usage-label">工具耗时</div>
          <div class="usage-detail">{{ formatDuration(latestAssistantCost.toolDurationMs) }}</div>
        </div>
      </div>
      <div
        v-else
        class="mt-3 text-sm text-muted-foreground"
      >
        还没有可展示的用量数据。
      </div>
    </div>
  </div>
</template>

<style scoped>
.usage-card {
  border: 1px solid #e7e2d7;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.8);
  padding: 14px;
}

.dark .usage-card {
  border-color: #333;
  background: #252525;
}

.usage-label {
  color: #8b816f;
  font-size: 11px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.usage-value {
  margin-top: 8px;
  color: #1a1a1a;
  font-size: 24px;
  font-weight: 600;
  line-height: 1.1;
}

.usage-detail {
  margin-top: 4px;
  color: #1a1a1a;
  font-size: 16px;
  font-weight: 600;
}

.dark .usage-value,
.dark .usage-detail {
  color: #ececec;
}
</style>
