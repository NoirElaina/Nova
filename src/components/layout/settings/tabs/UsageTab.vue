<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { LineChart } from 'echarts/charts';
import {
  GridComponent,
  LegendComponent,
  TooltipComponent,
  type GridComponentOption,
  type TooltipComponentOption,
} from 'echarts/components';
import * as echarts from 'echarts/core';
import type { ECharts, EChartsCoreOption } from 'echarts/core';
import { CanvasRenderer } from 'echarts/renderers';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../../../lib/chat-types';

echarts.use([LineChart, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer]);

const props = defineProps<{
  entries: ToolExecutionEntry[];
  messages: ChatMessage[];
  assistantTurnCost?: TurnCost;
}>();

type UsageChartOption = EChartsCoreOption & GridComponentOption & TooltipComponentOption;

const assistantMessages = computed(() =>
  props.messages.filter((message) => message.role === 'assistant'),
);

type TokenTurnPoint = {
  timestamp: number;
  index: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cumulativeTokens: number;
};

type TokenRange = 1 | 7 | 14 | 30 | 'all';

const selectedTokenRange = ref<TokenRange>('all');
const tokenRangeOptions: { label: string; value: TokenRange }[] = [
  { label: '今天', value: 1 },
  { label: '7 天', value: 7 },
  { label: '14 天', value: 14 },
  { label: '30 天', value: 30 },
  { label: '全部', value: 'all' },
];

const dateFilteredTokenPoints = computed<TokenTurnPoint[]>(() => {
  if (selectedTokenRange.value === 'all') return tokenTurnPoints.value;
  const now = Date.now();
  const cutoff = now - selectedTokenRange.value * 24 * 60 * 60 * 1000;
  return tokenTurnPoints.value.filter((p) => p.timestamp >= cutoff);
});

const visibleTokenTurnPoints = computed(() => {
  return dateFilteredTokenPoints.value;
});

const tokenTurnPoints = computed<TokenTurnPoint[]>(() => {
  let cumulativeTokens = 0;
  return assistantMessages.value
    .map((message, index) => {
      const inputTokens = Math.max(0, message.cost?.inputTokens ?? 0);
      const costOutputTokens = Math.max(0, message.cost?.outputTokens ?? 0);
      const fallbackOutputTokens =
        inputTokens + costOutputTokens > 0 ? 0 : Math.max(0, message.tokenUsage ?? 0);
      const outputTokens = costOutputTokens + fallbackOutputTokens;
      const totalTokens = inputTokens + outputTokens;
      cumulativeTokens += totalTokens;
      return {
        timestamp: Date.now(),
        index: index + 1,
        inputTokens,
        outputTokens,
        totalTokens,
        cumulativeTokens,
      };
    })
    .filter((point) => point.totalTokens > 0);
});

const totalInputTokens = computed(() =>
  tokenTurnPoints.value.reduce((sum, point) => sum + point.inputTokens, 0),
);

const totalOutputTokens = computed(() =>
  tokenTurnPoints.value.reduce((sum, point) => sum + point.outputTokens, 0),
);

const totalTokens = computed(() => totalInputTokens.value + totalOutputTokens.value);

const totalToolCalls = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.toolCalls ?? 0), 0),
);


const totalCacheReadTokens = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.cacheReadTokens ?? 0), 0),
);

const totalCacheCreationTokens = computed(() =>
  assistantMessages.value.reduce((sum, message) => sum + (message.cost?.cacheCreationTokens ?? 0), 0),
);

const totalCostUsd = computed(() => {
  let total = 0;
  for (const message of assistantMessages.value) {
    const cost = parseFloat(message.cost?.totalCostUsd ?? '0');
    if (!isNaN(cost)) total += cost;
  }
  return total;
});

const formatCost = (value: number) => {
  if (value <= 0) return '$0.00';
  if (value < 0.01) return '$' + value.toFixed(4);
  return '$' + value.toFixed(2);
};

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




const formatNumber = (value: number) => value.toLocaleString('en-US');
const formatDuration = (value: number) =>
  value >= 1000 ? `${(value / 1000).toFixed(1)} s` : `${value} ms`;

const formatCompactNumber = (value: number) => {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 10_000) return `${Math.round(value / 1_000)}k`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return `${value}`;
};

const perTurnValues = computed(() => visibleTokenTurnPoints.value.map((point) => point.totalTokens));
const perTurnMax = computed(() => Math.max(...perTurnValues.value, 1));
const latestTokenTurn = computed(() => visibleTokenTurnPoints.value[visibleTokenTurnPoints.value.length - 1] ?? null);
const averageTurnTokens = computed(() =>
  visibleTokenTurnPoints.value.length > 0
    ? Math.round(perTurnValues.value.reduce((sum, value) => sum + value, 0) / visibleTokenTurnPoints.value.length)
    : 0,
);

const inputTokenValues = computed(() => visibleTokenTurnPoints.value.map((p) => p.inputTokens));
const outputTokenValues = computed(() => visibleTokenTurnPoints.value.map((p) => p.outputTokens));
const cacheCreationValues = computed(() =>
  assistantMessages.value.slice(
    Math.max(0, assistantMessages.value.length - (selectedTokenRange.value === 'all' ? assistantMessages.value.length : selectedTokenRange.value))
  ).map((m) => m.cost?.cacheCreationTokens ?? 0)
);
const cacheReadValues = computed(() =>
  assistantMessages.value.slice(
    Math.max(0, assistantMessages.value.length - (selectedTokenRange.value === 'all' ? assistantMessages.value.length : selectedTokenRange.value))
  ).map((m) => m.cost?.cacheReadTokens ?? 0)
);

const costValues = computed(() =>
  visibleTokenTurnPoints.value.map((_, i) => {
    let total = 0;
    for (let j = 0; j <= i; j++) {
      const cost = parseFloat(assistantMessages.value[j]?.cost?.totalCostUsd ?? '0');
      if (!isNaN(cost)) total += cost;
    }
    return total;
  })
);

const turnLabels = computed(() => visibleTokenTurnPoints.value.map((point) => {
  const d = new Date(point.timestamp);
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  const hh = String(d.getHours()).padStart(2, '0');
  const mi = String(d.getMinutes()).padStart(2, '0');
  return `${mm}-${dd} ${hh}:${mi}`;
}));
const trendChartRef = ref<HTMLElement | null>(null);
let trendChart: ECharts | null = null;

const buildTrendOption = (): UsageChartOption => {
  const labels = turnLabels.value;
  return {
    animation: true,
    animationDuration: 600,
    animationEasing: 'cubicOut',
    grid: {
      left: 10,
      right: 50,
      top: 20,
      bottom: 62,
      containLabel: false,
    },
    legend: {
      data: ['Prompt', 'Output', 'Cache Write', 'Cache Hit', '累计成本'],
      bottom: 12,
      textStyle: { color: '#8b816f', fontSize: 11 },
      itemWidth: 10,
      itemHeight: 10,
      itemGap: 16,
    },
    tooltip: {
      trigger: 'axis',
      appendToBody: true,
      borderWidth: 0,
      backgroundColor: 'rgba(32, 28, 23, 0.94)',
      textStyle: { color: '#fff', fontSize: 12 },
      padding: [10, 12],
    },
    xAxis: {
      type: 'category',
      boundaryGap: false,
      data: labels,
      axisTick: { show: false },
      axisLine: { lineStyle: { color: 'rgba(201, 188, 170, 0.75)' } },
      axisLabel: { color: '#8b816f', fontSize: 10, hideOverlap: true, interval: 'auto' },
    },
    yAxis: [
      {
        type: 'value',
        min: 0,
        splitNumber: 3,
        axisLabel: { color: '#9a8f80', fontSize: 10, formatter: (v: number) => formatCompactNumber(v) },
        splitLine: { lineStyle: { color: 'rgba(226, 216, 201, 0.55)', type: 'dashed' } },
      },
      {
        type: 'value',
        min: 0,
        splitNumber: 3,
        axisLabel: { color: '#9a8f80', fontSize: 10, formatter: (v: number) => `$${v.toFixed(2)}` },
        splitLine: { show: false },
      },
    ],
    series: [
      {
        name: 'Prompt',
        type: 'line',
        yAxisIndex: 0,
        data: inputTokenValues.value,
        smooth: true,
        showSymbol: labels.length <= 12,
        symbol: 'circle',
        symbolSize: 5,
        lineStyle: { color: '#3b82f6', width: 2 },
        itemStyle: { color: '#3b82f6' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(59,130,246,0.18)' },
            { offset: 1, color: 'rgba(59,130,246,0)' },
          ]),
        },
      },
      {
        name: 'Output',
        type: 'line',
        yAxisIndex: 0,
        data: outputTokenValues.value,
        smooth: true,
        showSymbol: labels.length <= 12,
        symbol: 'circle',
        symbolSize: 5,
        lineStyle: { color: '#22c55e', width: 2 },
        itemStyle: { color: '#22c55e' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(34,197,94,0.18)' },
            { offset: 1, color: 'rgba(34,197,94,0)' },
          ]),
        },
      },
      {
        name: 'Cache Write',
        type: 'line',
        yAxisIndex: 0,
        data: cacheCreationValues.value,
        smooth: true,
        showSymbol: false,
        lineStyle: { color: '#f97316', width: 1.5 },
        itemStyle: { color: '#f97316' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(249,115,22,0.14)' },
            { offset: 1, color: 'rgba(249,115,22,0)' },
          ]),
        },
      },
      {
        name: 'Cache Hit',
        type: 'line',
        yAxisIndex: 0,
        data: cacheReadValues.value,
        smooth: true,
        showSymbol: false,
        lineStyle: { color: '#a855f7', width: 1.5 },
        itemStyle: { color: '#a855f7' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(168,85,247,0.14)' },
            { offset: 1, color: 'rgba(168,85,247,0)' },
          ]),
        },
      },
      {
        name: '累计成本',
        type: 'line',
        yAxisIndex: 1,
        data: costValues.value,
        smooth: true,
        showSymbol: false,
        lineStyle: { color: '#f43f5e', width: 2, type: 'dashed' },
        itemStyle: { color: '#f43f5e' },
        areaStyle: undefined,
      },
    ],
  };
};

const renderCharts = async () => {
  await nextTick();
  trendChart?.dispose();
  trendChart = null;
  if (!trendChartRef.value || visibleTokenTurnPoints.value.length === 0) return;
  trendChart = echarts.init(trendChartRef.value);
  trendChart.setOption(buildTrendOption(), true);
  trendChart.resize();
};

const resizeCharts = () => {
  trendChart?.resize();
};

watch(
  () => [
    selectedTokenRange.value,
    visibleTokenTurnPoints.value.length,
  ],
  () => {
    void renderCharts();
  },
  { immediate: true },
);

onMounted(() => {
  window.addEventListener('resize', resizeCharts);
  void renderCharts();
});

onBeforeUnmount(() => {
  window.removeEventListener('resize', resizeCharts);
  trendChart?.dispose();
  trendChart = null;
});
</script>

<template>
  <div class="h-full overflow-y-auto px-4 py-4">
    <!-- Chart -- top, largest -->
    <div class="usage-chart-card mb-4">
      <div class="usage-chart-header">
        <div class="usage-chart-metrics-new">
          <div class="usage-metric-block">
            <span class="usage-metric-label">最新</span>
            <strong class="usage-metric-value">{{ formatCompactNumber(latestTokenTurn?.totalTokens ?? 0) }}</strong>
          </div>
          <div class="usage-metric-block">
            <span class="usage-metric-label">峰值</span>
            <strong class="usage-metric-value">{{ formatCompactNumber(perTurnMax) }}</strong>
          </div>
          <div class="usage-metric-block">
            <span class="usage-metric-label">平均</span>
            <strong class="usage-metric-value">{{ formatCompactNumber(averageTurnTokens) }}</strong>
          </div>
        </div>
        <div class="flex items-center gap-3">
          <div class="usage-range-toggle" aria-label="区间">
            <button
              v-for="option in tokenRangeOptions"
              :key="String(option.value)"
              type="button"
              :class="{ active: selectedTokenRange === option.value }"
              @click="selectedTokenRange = option.value"
            >
              {{ option.label }}
            </button>
          </div>
          <div class="usage-trends-pill">{{ visibleTokenTurnPoints.length }}/{{ tokenTurnPoints.length }} 次</div>
        </div>
      </div>
      <div class="usage-chart-frame">
        <div v-if="visibleTokenTurnPoints.length > 0" ref="trendChartRef" class="usage-echart usage-echart--large" />
        <div v-else class="usage-chart-empty">还没有足够的 token 数据。</div>
      </div>
    </div>

    <!-- Stat cards -->
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
        <div class="usage-label">总费用</div>
        <div class="usage-value">{{ formatCost(totalCostUsd) }}</div>
      </div>
    </div>

    <div class="mt-4 grid grid-cols-2 gap-3 lg:grid-cols-3">
      <div class="usage-card">
        <div class="usage-label">Tool Calls</div>
        <div class="usage-value">{{ formatNumber(totalToolCalls) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">Cache Read</div>
        <div class="usage-value">{{ formatNumber(totalCacheReadTokens) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">Cache Write</div>
        <div class="usage-value">{{ formatNumber(totalCacheCreationTokens) }}</div>
      </div>
    </div>

    <div class="mt-4 rounded-xl border border-[#e7e2d7] bg-white/80 p-4 dark:border-[#333] dark:bg-[#252525]">
      <div class="text-sm font-medium text-[#1a1a1a] dark:text-[#ececec]">最近一次</div>
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
        <div>
          <div class="usage-label">费用</div>
          <div class="usage-detail">{{ formatCost(parseFloat(latestAssistantCost.totalCostUsd ?? '0')) }}</div>
        </div>
        <div>
          <div class="usage-label">Input 费用</div>
          <div class="usage-detail">{{ formatCost(parseFloat(latestAssistantCost.inputCostUsd ?? '0')) }}</div>
        </div>
        <div>
          <div class="usage-label">Output 费用</div>
          <div class="usage-detail">{{ formatCost(parseFloat(latestAssistantCost.outputCostUsd ?? '0')) }}</div>
        </div>
        <div>
          <div class="usage-label">Cache Read</div>
          <div class="usage-detail">{{ formatNumber(latestAssistantCost.cacheReadTokens ?? 0) }}</div>
        </div>
        <div>
          <div class="usage-label">Cache Write</div>
          <div class="usage-detail">{{ formatNumber(latestAssistantCost.cacheCreationTokens ?? 0) }}</div>
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

.usage-trends-pill {
  flex-shrink: 0;
  background: rgba(255, 255, 255, 0.72);
  color: #7d725f;
  font-size: 12px;
  padding: 5px 10px;
}

.usage-range-toggle {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  border: 1px solid #e1d8c8;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.62);
  padding: 3px;
}

.usage-range-toggle button {
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: #847764;
  cursor: pointer;
  font-size: 12px;
  line-height: 1;
  padding: 6px 9px;
  transition:
    background 160ms ease,
    color 160ms ease,
    box-shadow 160ms ease;
}

.usage-range-toggle button:hover {
  background: rgba(235, 228, 216, 0.72);
  color: #4f473c;
}

.usage-range-toggle button.active {
  background: #1f1f1d;
  box-shadow: 0 5px 14px rgba(55, 45, 31, 0.16);
  color: #fffaf0;
}

.dark .usage-trends-pill {
  border-color: #46413a;
  background: rgba(255, 255, 255, 0.05);
  color: #b9b0a2;
}

.dark .usage-range-toggle {
  border-color: #46413a;
  background: rgba(255, 255, 255, 0.04);
}

.dark .usage-range-toggle button {
  color: #b9b0a2;
}

.dark .usage-range-toggle button:hover {
  background: rgba(255, 255, 255, 0.07);
  color: #eee4d6;
}

.dark .usage-range-toggle button.active {
  background: #eee4d6;
  color: #1f1f1d;
}

.usage-chart-card {
  position: relative;
  overflow: hidden;
  border: 1px solid rgba(226, 216, 201, 0.88);
  border-radius: 18px;
  background: rgba(255, 255, 255, 0.72);
  padding: 15px 16px 12px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.75);
}

.dark .usage-chart-card {
  border-color: rgba(67, 62, 55, 0.9);
  background: rgba(35, 35, 35, 0.72);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
}

.usage-chart-header {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.usage-chart-kicker {
  color: #9a8b75;
  font-size: 11px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.usage-chart-title {
  margin-top: 3px;
  color: #1a1a1a;
  font-size: 15px;
  font-weight: 680;
  letter-spacing: -0.01em;
}

.dark .usage-chart-title {
  color: #ececec;
}

.dark .usage-chart-kicker {
  color: #a99f90;
}

.usage-chart-empty {
  color: #8b816f;
  font-size: 11px;
}

.usage-chart-metrics-new {
  display: flex;
  gap: 6px;
}

.usage-metric-block {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 4px 10px;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  background: #f9fafb;
}

.usage-metric-label {
  color: #6b7280;
  font-size: 11px;
}

.usage-metric-value {
  color: #111827;
  font-size: 15px;
  font-weight: 700;
}

.dark .usage-metric-block {
  border-color: #374151;
  background: #1f2937;
}

.dark .usage-metric-label {
  color: #9ca3af;
}

.dark .usage-metric-value {
  color: #f3f4f6;
}

.usage-chart-metrics {
  display: flex;
  gap: 8px;
}

.usage-chart-metrics > div {
  min-width: 58px;
  border: 1px solid rgba(226, 216, 201, 0.72);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.58);
  padding: 7px 8px;
  text-align: right;
}

.usage-chart-metrics span {
  display: block;
  color: #9a8b75;
  font-size: 10px;
  line-height: 1;
}

.usage-chart-metrics strong {
  display: block;
  margin-top: 5px;
  color: #1a1a1a;
  font-size: 16px;
  font-weight: 700;
  line-height: 1;
}



.usage-chart-frame {
  position: relative;
  z-index: 1;
  margin-top: 14px;
}

.usage-echart {
  width: 100%;
  height: 142px;
}

.usage-echart--large {
  height: 320px;
}

.usage-chart-empty {
  margin-top: 16px;
  border: 1px dashed #e2dacc;
  border-radius: 12px;
  padding: 18px;
  text-align: center;
}

.dark .usage-chart-empty {
  color: #a99f90;
}

</style>

<style>
.usage-echart-tooltip {
  min-width: 118px;
  font-family: inherit;
  line-height: 1.5;
}

.usage-echart-tooltip__title {
  margin-bottom: 3px;
  color: rgba(255, 255, 255, 0.72);
  font-size: 11px;
}
</style>
