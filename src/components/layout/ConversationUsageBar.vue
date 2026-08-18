<script setup lang="ts">
import { computed } from 'vue';
import type { ConversationUsageSummary } from '../../lib/chat-types';

const props = defineProps<{
  usage?: ConversationUsageSummary | null;
}>();

// 总输入 = 未缓存输入 + 缓存写入 + 缓存读取（与 Anthropic usage 语义一致）
const totalInputTokens = computed(
  () =>
    (props.usage?.inputTokens ?? 0) +
    (props.usage?.cacheCreationTokens ?? 0) +
    (props.usage?.cacheReadTokens ?? 0),
);

const totalOutputTokens = computed(() => props.usage?.outputTokens ?? 0);

// 缓存命中率 = 缓存读取 / 总输入。只在会话确实产生过缓存活动时展示，
// 避免不支持缓存的模型显示误导性的 0%。
const cacheHitRate = computed<number | null>(() => {
  const cacheRead = props.usage?.cacheReadTokens ?? 0;
  const cacheCreation = props.usage?.cacheCreationTokens ?? 0;
  if (cacheRead + cacheCreation <= 0) return null;
  if (totalInputTokens.value <= 0) return null;
  return cacheRead / totalInputTokens.value;
});

const costText = computed<string | null>(() => {
  const raw = parseFloat(props.usage?.totalCostUsd ?? '');
  if (!Number.isFinite(raw) || raw <= 0) return null;
  // 小额成本保留 4 位小数，避免显示成 $0.00
  return raw < 0.01 ? raw.toFixed(4) : raw.toFixed(2);
});

const hasAnyUsage = computed(() => totalInputTokens.value > 0 || totalOutputTokens.value > 0);

const formatTokens = (value: number) => {
  const rounded = Math.max(0, Math.round(value));
  if (rounded >= 1_000_000) {
    return `${(rounded / 1_000_000).toFixed(rounded >= 10_000_000 ? 0 : 1)}M`;
  }
  if (rounded >= 1_000) {
    return `${(rounded / 1_000).toFixed(rounded >= 100_000 ? 0 : 1)}k`;
  }
  return String(rounded);
};
</script>

<template>
  <div class="usage-bar" aria-label="会话用量统计">
    <template v-if="hasAnyUsage">
      <span class="usage-item">
        <svg class="usage-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
          stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="12" y1="19" x2="12" y2="5" />
          <polyline points="5 12 12 5 19 12" />
        </svg>
        {{ formatTokens(totalInputTokens) }}
      </span>
      <span class="usage-item">
        <svg class="usage-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
          stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="12" y1="5" x2="12" y2="19" />
          <polyline points="19 12 12 19 5 12" />
        </svg>
        {{ formatTokens(totalOutputTokens) }}
      </span>
      <span v-if="cacheHitRate !== null" class="usage-item usage-cache">
        缓存命中 {{ (cacheHitRate * 100).toFixed(1).replace(/\.0$/, '') }}%
      </span>
      <span v-if="costText" class="usage-item usage-cost">${{ costText }}</span>
    </template>
    <span v-else class="usage-empty">暂无用量</span>
  </div>
</template>

<style scoped>
.usage-bar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
  min-height: 18px;
  font-size: 12px;
  line-height: 1.2;
  color: #8b929d;
  font-variant-numeric: tabular-nums;
  user-select: none;
}

.usage-item {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  white-space: nowrap;
}

.usage-icon {
  width: 12px;
  height: 12px;
  opacity: 0.75;
}

.usage-cache {
  color: #7a9e7e;
}

.usage-cost {
  color: #b08860;
}

.usage-empty {
  opacity: 0.5;
}

.dark .usage-bar {
  color: #a7a19a;
}

.dark .usage-cache {
  color: #9ec2a2;
}

.dark .usage-cost {
  color: #d0a878;
}
</style>
