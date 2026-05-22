<script setup lang="ts">
import type { ContextCompactSummary } from '../../../lib/chat-types';

const props = defineProps<{
  items: ContextCompactSummary[];
  compact?: boolean;
}>();

const formatTokens = (value?: number) => {
  if (typeof value !== 'number' || value <= 0) {
    return '0';
  }
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}m`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}k`;
  }
  return String(Math.round(value));
};

const buildLine = (item: ContextCompactSummary) => {
  const reason = item.reason?.trim() || '已自动压缩上下文';
  if ((item.beforeTokens ?? 0) > 0 || (item.afterTokens ?? 0) > 0) {
    return `${reason} · ${formatTokens(item.beforeTokens)} -> ${formatTokens(item.afterTokens)} · -${formatTokens(item.savedTokens)} tokens`;
  }
  return `${reason} · -${formatTokens(item.savedTokens)} tokens`;
};

void props;
</script>

<template>
  <div class="compact-notice" :class="{ 'is-compact': compact }">
    <div class="compact-notice-title">上下文已压缩</div>
    <div class="compact-notice-list">
      <p
        v-for="(item, index) in items"
        :key="`${item.level || 'compact'}-${index}`"
        class="compact-notice-line"
      >
        {{ buildLine(item) }}
      </p>
    </div>
  </div>
</template>

<style scoped>
.compact-notice {
  margin: 10px 0 12px;
  padding: 10px 11px;
  border-radius: 10px;
  border: 1px solid rgba(203, 213, 225, 0.82);
  background: rgba(248, 250, 252, 0.9);
}

.compact-notice.is-compact {
  margin-top: 8px;
  padding: 8px 10px;
}

.compact-notice-title {
  font-size: 11px;
  font-weight: 600;
  color: #475569;
  margin-bottom: 4px;
}

.compact-notice-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.compact-notice-line {
  margin: 0;
  font-size: 11px;
  line-height: 1.45;
  color: #64748b;
}

.dark .compact-notice {
  border-color: #3f4652;
  background: rgba(31, 41, 55, 0.86);
}

.dark .compact-notice-title {
  color: #e2e8f0;
}

.dark .compact-notice-line {
  color: #cbd5e1;
}
</style>
