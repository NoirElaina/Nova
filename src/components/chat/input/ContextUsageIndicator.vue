<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { onClickOutside } from '@vueuse/core';
import { RotateCw, Sparkles } from 'lucide-vue-next';
import type { ContextUsage } from '@/lib/chat-types';

const props = defineProps<{
  usage?: ContextUsage;
  usedTokens?: number;
  model?: string;
  compacting?: boolean;
}>();

const emit = defineEmits<{
  (e: 'compact'): void;
}>();

const rootRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);

onClickOutside(rootRef, () => {
  isOpen.value = false;
});

const togglePopover = () => {
  isOpen.value = !isOpen.value;
};

const handleCompactClick = () => {
  if (props.compacting) return;
  emit('compact');
};

const DEFAULT_WINDOW_TOKENS = 200_000;

// 当没有 usage.windowTokens 时，从后端按模型名查询窗口大小。
const modelWindowTokens = ref<number>(DEFAULT_WINDOW_TOKENS);

watch(
  () => props.model,
  async (model) => {
    if (!model) return;
    try {
      const v = await invoke<number>('get_model_window_tokens', { model });
      if (v > 0) modelWindowTokens.value = v;
    } catch {
      // 查询失败时保留 default，不影响显示
    }
  },
  { immediate: true }
);

const resolvedUsage = computed<ContextUsage>(() => ({
  usedTokens: Math.max(0, Math.round(props.usage?.usedTokens ?? props.usedTokens ?? 0)),
  windowTokens: props.usage?.windowTokens ?? modelWindowTokens.value,
  responseReserveTokens: props.usage?.responseReserveTokens ?? 0,
  source: props.usage?.source,
}));

const usedTokens = computed(() => resolvedUsage.value.usedTokens);
const windowTokens = computed(() => Math.max(1, Math.round(resolvedUsage.value.windowTokens ?? modelWindowTokens.value)));
const responseReserveTokens = computed(() => Math.max(0, Math.round(resolvedUsage.value.responseReserveTokens ?? 0)));
const usedPercent = computed(() => Math.min(100, Math.round((usedTokens.value / windowTokens.value) * 100)));
const barPercent = computed(() => Math.min(100, Math.max(0, (usedTokens.value / windowTokens.value) * 100)));
const reservePercent = computed(() => Math.min(100, Math.max(0, (responseReserveTokens.value / windowTokens.value) * 100)));

const RING_RADIUS = 6.2;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS; // ≈ 38.96
const ringOffset = computed(() => {
  const filled = (barPercent.value / 100) * RING_CIRCUMFERENCE;
  return RING_CIRCUMFERENCE - filled;
});
const ringColor = computed(() => {
  const p = barPercent.value;
  if (p >= 90) return 'var(--ring-danger, #d04f2a)';
  if (p >= 70) return 'var(--ring-warn, #d57956)';
  return 'currentColor';
});

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
  <div ref="rootRef" class="context-usage-root">
    <button
      type="button"
      class="context-usage-button"
      :class="{ 'is-active': isOpen, 'is-compacting': compacting }"
      :aria-label="`上下文已用 ${formatTokens(usedTokens)} 个令牌`"
      :title="`上下文窗口: ${formatTokens(usedTokens)} / ${formatTokens(windowTokens)} (${usedPercent}%)`"
      @click="togglePopover"
    >
      <svg
        class="context-usage-ring"
        :class="{ 'animate-spin': compacting }"
        width="18"
        height="18"
        viewBox="0 0 18 18"
        aria-hidden="true"
      >
        <!-- 轨道圆 -->
        <circle class="ring-track" cx="9" cy="9" r="6.2" />
        <!-- 进度圆：从顶部 (-90°) 顺时针填充 -->
        <circle
          class="ring-progress"
          cx="9"
          cy="9"
          r="6.2"
          :stroke="ringColor"
          :stroke-dasharray="RING_CIRCUMFERENCE"
          :stroke-dashoffset="ringOffset"
        />
      </svg>
    </button>

    <div
      class="context-usage-popover"
      :class="{ 'is-open': isOpen }"
    >
      <div class="context-title flex items-center justify-between">
        <span>上下文窗口</span>
        <span class="text-[11px] font-mono text-muted-foreground">{{ usedPercent }}%</span>
      </div>

      <div class="context-summary">
        <span>{{ formatTokens(usedTokens) }} / {{ formatTokens(windowTokens) }} 令牌</span>
      </div>

      <div class="context-bar">
        <div class="context-bar-fill" :style="{ width: `${barPercent}%` }"></div>
        <div
          v-if="responseReserveTokens > 0"
          class="context-bar-reserve"
          :style="{ width: `${reservePercent}%` }"
        ></div>
      </div>

      <div class="reserve-row">
        <span class="reserve-mark"></span>
        <span>保留用于模型回复</span>
      </div>

      <button
        type="button"
        class="compact-button flex items-center justify-center gap-1.5 cursor-pointer font-medium"
        :disabled="compacting"
        @click="handleCompactClick"
      >
        <RotateCw v-if="compacting" class="w-3.5 h-3.5 animate-spin text-blue-600 dark:text-blue-400" />
        <Sparkles v-else class="w-3.5 h-3.5 text-amber-500" />
        <span>{{ compacting ? '正在压缩上下文…' : '立即压缩对话历史' }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.context-usage-root {
  position: relative;
  display: flex;
  align-items: center;
}

.context-usage-button {
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 999px;
  color: #8b929d;
  transition: background-color 160ms ease, color 160ms ease;
  cursor: pointer;
}

.context-usage-button:hover,
.context-usage-button.is-active {
  background: rgba(15, 23, 42, 0.055);
  color: #596273;
}

.dark .context-usage-button {
  color: #a7a19a;
}

.dark .context-usage-button:hover,
.dark .context-usage-button.is-active {
  background: rgba(255, 255, 255, 0.08);
  color: #d8d3ca;
}

.context-usage-ring {
  fill: none;
  stroke-linecap: round;
  /* 让进度从 12 点钟方向开始顺时针填充 */
  transform: rotate(-90deg);
  transform-origin: center;
}

.ring-track {
  stroke: currentColor;
  stroke-width: 2;
  opacity: 0.18;
}

.ring-progress {
  stroke-width: 2.4;
  opacity: 0.9;
  transition: stroke-dashoffset 600ms cubic-bezier(0.4, 0, 0.2, 1),
              stroke 400ms ease;
}

.context-usage-popover {
  position: absolute;
  right: -18px;
  bottom: 34px;
  z-index: 50;
  width: 250px;
  padding: 12px;
  border-radius: 14px;
  border: 1px solid rgba(229, 231, 235, 0.96);
  background: rgba(255, 255, 255, 0.98);
  color: #667085;
  box-shadow: 0 16px 36px rgba(15, 23, 42, 0.12);
  opacity: 0;
  transform: translateY(4px);
  pointer-events: none;
  transition: opacity 150ms ease, transform 150ms ease;
}

.context-usage-popover.is-open {
  opacity: 1;
  transform: translateY(0);
  pointer-events: auto;
}

.context-usage-popover::after {
  content: '';
  position: absolute;
  right: 21px;
  bottom: -7px;
  width: 12px;
  height: 12px;
  border-right: 1px solid rgba(229, 231, 235, 0.96);
  border-bottom: 1px solid rgba(229, 231, 235, 0.96);
  background: rgba(255, 255, 255, 0.98);
  transform: rotate(45deg);
}

.context-title {
  font-size: 13px;
  font-weight: 650;
  color: #334155;
  line-height: 1.2;
}

.context-summary {
  margin-top: 6px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  line-height: 1.2;
  color: #64748b;
  font-variant-numeric: tabular-nums;
}

.context-bar {
  position: relative;
  margin-top: 8px;
  height: 5px;
  overflow: hidden;
  border-radius: 999px;
  background: rgba(229, 231, 235, 0.84);
}

.context-bar-fill {
  height: 100%;
  border-radius: inherit;
  background: #3b82f6;
  transition: width 300ms ease;
}

.context-bar-reserve {
  position: absolute;
  top: 0;
  right: 0;
  height: 100%;
  border-radius: inherit;
  background: repeating-linear-gradient(
    135deg,
    #93c5fa 0,
    #93c5fa 3px,
    transparent 3px,
    transparent 6px
  );
}

.reserve-row {
  margin-top: 8px;
  display: flex;
  align-items: center;
  gap: 8px;
  color: #94a3b8;
  font-size: 11px;
}

.reserve-mark {
  width: 14px;
  height: 8px;
  border-radius: 2px;
  background: repeating-linear-gradient(
    135deg,
    #93c5fa 0,
    #93c5fa 3px,
    transparent 3px,
    transparent 6px
  );
}

.compact-button {
  width: 100%;
  margin-top: 12px;
  height: 32px;
  border-radius: 9px;
  border: 1px solid rgba(226, 232, 240, 0.96);
  color: #334155;
  background: #f8fafc;
  font-size: 12px;
  transition: background-color 140ms ease, border-color 140ms ease, transform 100ms ease;
}

.compact-button:hover:not(:disabled) {
  background: #f1f5f9;
  border-color: rgba(203, 213, 225, 0.96);
  transform: translateY(-0.5px);
}

.compact-button:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.dark .context-usage-popover {
  border-color: rgba(63, 63, 70, 0.98);
  background: rgba(24, 24, 27, 0.98);
  color: #d4d4d8;
  box-shadow: 0 18px 42px rgba(0, 0, 0, 0.4);
}

.dark .context-usage-popover::after {
  border-color: rgba(63, 63, 70, 0.98);
  background: rgba(24, 24, 27, 0.98);
}

.dark .context-title {
  color: #f4f4f5;
}

.dark .context-summary {
  color: #a1a1aa;
}

.dark .reserve-row {
  color: #71717a;
}

.dark .context-bar {
  background: rgba(63, 63, 70, 0.8);
}

.dark .compact-button {
  color: #f4f4f5;
  border-color: rgba(63, 63, 70, 0.95);
  background: rgba(39, 39, 42, 0.85);
}

.dark .compact-button:hover:not(:disabled) {
  background: rgba(63, 63, 70, 0.95);
  border-color: rgba(82, 82, 91, 0.95);
}
</style>
