<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { ContextUsage } from '../../lib/chat-types';

const props = defineProps<{
  usage?: ContextUsage;
  usedTokens?: number;
  model?: string;
}>();

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
  breakdown: props.usage?.breakdown,
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

const percentOfWindow = (value?: number) => {
  if (!value || value <= 0) {
    return '0%';
  }
  const percent = (value / windowTokens.value) * 100;
  return `${percent < 0.1 ? '<0.1' : percent.toFixed(1)}%`;
};

const breakdownRows = computed(() => {
  const breakdown = resolvedUsage.value.breakdown ?? {};
  return [
    {
      section: 'System',
      rows: [
        { label: 'System Instructions', value: breakdown.systemInstructions ?? 0 },
        { label: 'Tool Definitions', value: breakdown.toolDefinitions ?? 0 },
      ],
    },
    {
      section: 'User Context',
      rows: [
        { label: 'Messages', value: breakdown.messages ?? 0 },
        { label: 'Tool Results', value: breakdown.toolResults ?? 0 },
      ],
    },
    {
      section: '未分类',
      rows: [
        { label: '其他', value: breakdown.other ?? 0 },
      ],
    },
  ];
});
</script>

<template>
  <div class="context-usage-root">
    <button
      type="button"
      class="context-usage-button"
      :aria-label="`上下文已用 ${formatTokens(usedTokens)} 个令牌`"
    >
      <svg class="context-usage-ring" width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
        <!-- 轨道圆 -->
        <circle class="ring-track" cx="9" cy="9" r="6.2" />
        <!-- 进度圆：从顶部 (-90°) 顺时针填充 -->
        <circle
          class="ring-progress"
          cx="9" cy="9" r="6.2"
          :stroke="ringColor"
          :stroke-dasharray="RING_CIRCUMFERENCE"
          :stroke-dashoffset="ringOffset"
        />
      </svg>
    </button>

    <div class="context-usage-popover">
      <div class="context-title">上下文窗口</div>
      <div class="context-summary">
        <span>{{ formatTokens(usedTokens) }}/{{ formatTokens(windowTokens) }} 个令牌</span>
        <span>{{ usedPercent }}%</span>
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
        <span>保留用于响应</span>
      </div>

      <div v-for="group in breakdownRows" :key="group.section" class="context-section">
        <div class="context-section-title">{{ group.section }}</div>
        <div v-for="row in group.rows" :key="row.label" class="context-row">
          <span>{{ row.label }}</span>
          <span>{{ percentOfWindow(row.value) }}</span>
        </div>
      </div>

      <button type="button" class="compact-button">
        压缩对话
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
  color: #7f7970;
  transition: background-color 160ms ease, color 160ms ease;
}

.context-usage-button:hover {
  background: rgba(120, 112, 100, 0.1);
  color: #4f4941;
}

.dark .context-usage-button {
  color: #a7a19a;
}

.dark .context-usage-button:hover {
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
  bottom: 32px;
  z-index: 50;
  width: 242px;
  padding: 11px 11px 10px;
  border-radius: 13px;
  border: 1px solid rgba(221, 213, 199, 0.95);
  background: rgba(255, 252, 246, 0.98);
  color: #625a4d;
  box-shadow: 0 18px 42px rgba(68, 55, 36, 0.14);
  opacity: 0;
  transform: translateY(4px);
  pointer-events: none;
  transition: opacity 120ms ease, transform 120ms ease;
}

.context-usage-popover::after {
  content: '';
  position: absolute;
  right: 21px;
  bottom: -7px;
  width: 12px;
  height: 12px;
  border-right: 1px solid rgba(221, 213, 199, 0.95);
  border-bottom: 1px solid rgba(221, 213, 199, 0.95);
  background: rgba(255, 252, 246, 0.98);
  transform: rotate(45deg);
}

.context-usage-root:hover .context-usage-popover,
.context-usage-root:focus-within .context-usage-popover {
  opacity: 1;
  transform: translateY(0);
  pointer-events: auto;
}

.context-title {
  font-size: 13px;
  font-weight: 650;
  color: #6a6256;
  line-height: 1.2;
}

.context-summary {
  margin-top: 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  line-height: 1.2;
  color: #4f473b;
  font-variant-numeric: tabular-nums;
}

.context-bar {
  position: relative;
  margin-top: 7px;
  height: 4px;
  overflow: hidden;
  border-radius: 999px;
  background: rgba(214, 205, 190, 0.72);
}

.context-bar-fill {
  height: 100%;
  border-radius: inherit;
  background: #d57956;
}

.context-bar-reserve {
  position: absolute;
  top: 0;
  right: 0;
  height: 100%;
  border-radius: inherit;
  background: repeating-linear-gradient(
    135deg,
    #d57956 0,
    #d57956 3px,
    transparent 3px,
    transparent 6px
  );
}

.reserve-row {
  margin-top: 9px;
  display: flex;
  align-items: center;
  gap: 8px;
  color: #8c8376;
  font-size: 11px;
}

.reserve-mark {
  width: 15px;
  height: 10px;
  border-radius: 2px;
  background: repeating-linear-gradient(
    135deg,
    #d57956 0,
    #d57956 3px,
    transparent 3px,
    transparent 6px
  );
}

.context-section {
  margin-top: 12px;
}

.context-section-title {
  margin-bottom: 6px;
  color: #8c8376;
  font-size: 11px;
  font-weight: 700;
}

.context-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 5px;
  color: #574f44;
  font-size: 12px;
  line-height: 1.25;
}

.context-row span:last-child {
  color: #9b9286;
  font-variant-numeric: tabular-nums;
}

.compact-button {
  width: 100%;
  margin-top: 12px;
  height: 30px;
  border-radius: 9px;
  border: 1px solid rgba(218, 211, 199, 0.95);
  color: #6a5f50;
  background: rgba(247, 243, 236, 0.82);
  font-size: 12px;
  transition: background-color 140ms ease, border-color 140ms ease;
}

.compact-button:hover {
  background: rgba(241, 234, 223, 0.95);
  border-color: rgba(205, 195, 178, 0.95);
}

.dark .context-usage-popover {
  border-color: rgba(71, 66, 58, 0.98);
  background: rgba(43, 42, 39, 0.98);
  color: #c8c0b4;
  box-shadow: 0 18px 42px rgba(0, 0, 0, 0.28);
}

.dark .context-usage-popover::after {
  border-color: rgba(71, 66, 58, 0.98);
  background: rgba(43, 42, 39, 0.98);
}

.dark .context-title,
.dark .context-summary,
.dark .context-row {
  color: #ddd5c7;
}

.dark .context-section-title,
.dark .reserve-row,
.dark .context-row span:last-child {
  color: #a79f92;
}

.dark .context-bar {
  background: rgba(81, 76, 68, 0.8);
}

.dark .compact-button {
  color: #ddd5c7;
  border-color: rgba(74, 69, 61, 0.95);
  background: rgba(52, 50, 46, 0.85);
}

.dark .compact-button:hover {
  background: rgba(62, 59, 54, 0.95);
  border-color: rgba(89, 82, 72, 0.95);
}
</style>
