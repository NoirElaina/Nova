<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { LineChart } from 'echarts/charts';
import {
  GridComponent,
  TooltipComponent,
  type GridComponentOption,
  type TooltipComponentOption,
} from 'echarts/components';
import * as echarts from 'echarts/core';
import type { ECharts, EChartsCoreOption } from 'echarts/core';
import { CanvasRenderer } from 'echarts/renderers';
import type { ChatMessage, ToolExecutionEntry, TurnCost } from '../../../lib/chat-types';

echarts.use([LineChart, GridComponent, TooltipComponent, CanvasRenderer]);

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
  index: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cumulativeTokens: number;
};

type TokenRange = 5 | 8 | 'all';

const selectedTokenRange = ref<TokenRange>(8);
const tokenRangeOptions: { label: string; value: TokenRange }[] = [
  { label: '最近 5 轮', value: 5 },
  { label: '最近 8 轮', value: 8 },
  { label: '全部', value: 'all' },
];

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
        index: index + 1,
        inputTokens,
        outputTokens,
        totalTokens,
        cumulativeTokens,
      };
    })
    .filter((point) => point.totalTokens > 0);
});

const visibleTokenTurnPoints = computed(() => {
  if (selectedTokenRange.value === 'all') {
    return tokenTurnPoints.value;
  }
  return tokenTurnPoints.value.slice(-selectedTokenRange.value);
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

const visibleInputTokens = computed(() =>
  visibleTokenTurnPoints.value.reduce((sum, point) => sum + point.inputTokens, 0),
);

const visibleOutputTokens = computed(() =>
  visibleTokenTurnPoints.value.reduce((sum, point) => sum + point.outputTokens, 0),
);

const perTurnValues = computed(() => visibleTokenTurnPoints.value.map((point) => point.totalTokens));
const cumulativeValues = computed(() => visibleTokenTurnPoints.value.map((point) => point.cumulativeTokens));
const perTurnMax = computed(() => Math.max(...perTurnValues.value, 1));
const latestTokenTurn = computed(() => visibleTokenTurnPoints.value[visibleTokenTurnPoints.value.length - 1] ?? null);
const averageTurnTokens = computed(() =>
  visibleTokenTurnPoints.value.length > 0
    ? Math.round(perTurnValues.value.reduce((sum, value) => sum + value, 0) / visibleTokenTurnPoints.value.length)
    : 0,
);

const turnLabels = computed(() => visibleTokenTurnPoints.value.map((point) => `第 ${point.index} 轮`));
const perTurnChartRef = ref<HTMLElement | null>(null);
const cumulativeChartRef = ref<HTMLElement | null>(null);
let perTurnChart: ECharts | null = null;
let cumulativeChart: ECharts | null = null;

const tooltipFormatter = (params: unknown) => {
  const item = Array.isArray(params) ? params[0] : params;
  const point = item as { axisValueLabel?: string; value?: number; marker?: string; seriesName?: string };
  return `
    <div class="usage-echart-tooltip">
      <div class="usage-echart-tooltip__title">${point.axisValueLabel ?? ''}</div>
      <div>${point.marker ?? ''}${point.seriesName ?? 'Token'}：<b>${formatNumber(Number(point.value ?? 0))}</b></div>
    </div>
  `;
};

const buildChartOption = (
  title: string,
  values: number[],
  color: string,
  areaColor: string,
): UsageChartOption => ({
  animation: true,
  animationDuration: 700,
  animationEasing: 'cubicOut',
  grid: {
    left: 8,
    right: 12,
    top: 18,
    bottom: 26,
    containLabel: false,
  },
  tooltip: {
    trigger: 'axis',
    appendToBody: true,
    borderWidth: 0,
    backgroundColor: 'rgba(32, 28, 23, 0.92)',
    textStyle: {
      color: '#fff',
      fontSize: 12,
    },
    padding: [8, 10],
    axisPointer: {
      type: 'line',
      lineStyle: {
        color,
        width: 1,
        type: 'dashed',
        opacity: 0.7,
      },
    },
    formatter: tooltipFormatter,
  },
  xAxis: {
    type: 'category',
    boundaryGap: false,
    data: turnLabels.value,
    axisTick: { show: false },
    axisLine: { lineStyle: { color: 'rgba(201, 188, 170, 0.75)' } },
    axisLabel: {
      color: '#8b816f',
      fontSize: 10,
      hideOverlap: true,
      interval: 'auto',
    },
  },
  yAxis: {
    type: 'value',
    min: 0,
    splitNumber: 3,
    axisLabel: {
      color: '#9a8f80',
      fontSize: 10,
      formatter: (value: number) => formatCompactNumber(value),
    },
    splitLine: {
      lineStyle: {
        color: 'rgba(226, 216, 201, 0.65)',
        type: 'dashed',
      },
    },
  },
  series: [
    {
      name: title,
      type: 'line',
      data: values,
      smooth: true,
      showSymbol: values.length <= 12,
      symbol: 'circle',
      symbolSize: 7,
      lineStyle: {
        color,
        width: 2.5,
        shadowColor: areaColor,
        shadowBlur: 10,
      },
      itemStyle: {
        color,
        borderColor: '#fff',
        borderWidth: 1.5,
      },
      areaStyle: {
        color: areaColor,
        opacity: 0.38,
      },
      emphasis: {
        focus: 'series',
        scale: true,
        itemStyle: {
          borderWidth: 2.5,
          shadowBlur: 12,
          shadowColor: areaColor,
        },
      },
    },
  ],
});

const renderCharts = async () => {
  await nextTick();
  if (!perTurnChartRef.value || !cumulativeChartRef.value || visibleTokenTurnPoints.value.length === 0) {
    return;
  }

  perTurnChart ??= echarts.init(perTurnChartRef.value);
  cumulativeChart ??= echarts.init(cumulativeChartRef.value);
  perTurnChart.setOption(buildChartOption('每轮 Token', perTurnValues.value, '#a86f38', 'rgba(168, 111, 56, 0.35)'), true);
  cumulativeChart.setOption(
    buildChartOption('累计 Token', cumulativeValues.value, '#3d7f72', 'rgba(61, 127, 114, 0.34)'),
    true,
  );
  perTurnChart.resize();
  cumulativeChart.resize();
};

const resizeCharts = () => {
  perTurnChart?.resize();
  cumulativeChart?.resize();
};

watch(
  () => [
    selectedTokenRange.value,
    visibleTokenTurnPoints.value.length,
    perTurnValues.value.join(','),
    cumulativeValues.value.join(','),
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
  perTurnChart?.dispose();
  cumulativeChart?.dispose();
  perTurnChart = null;
  cumulativeChart = null;
});
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

    <div class="usage-trends-panel">
      <div class="usage-trends-header">
        <div>
          <div class="usage-trends-title">Token 趋势</div>
          <div class="usage-trends-subtitle">按 assistant 回合统计，展示单轮消耗与会话累计消耗。</div>
        </div>
        <div class="usage-trends-actions">
          <div class="usage-range-toggle" aria-label="Token 趋势区间">
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
          <div class="usage-trends-pill">{{ visibleTokenTurnPoints.length }}/{{ tokenTurnPoints.length }} 轮</div>
        </div>
      </div>

      <div v-if="visibleTokenTurnPoints.length > 0" class="usage-trend-grid">
        <div class="usage-chart-card usage-chart-card--turn">
          <div class="usage-chart-header">
            <div>
              <div class="usage-chart-kicker">每轮消耗</div>
              <div class="usage-chart-title">单轮 Token</div>
            </div>
            <div class="usage-chart-metrics">
              <div>
                <span>最新</span>
                <strong>{{ formatCompactNumber(latestTokenTurn?.totalTokens ?? 0) }}</strong>
              </div>
              <div>
                <span>峰值</span>
                <strong>{{ formatCompactNumber(perTurnMax) }}</strong>
              </div>
            </div>
          </div>
          <div class="usage-chart-frame">
            <div ref="perTurnChartRef" class="usage-echart" />
            <div class="usage-chart-caption">
              <span>第 {{ visibleTokenTurnPoints[0]?.index ?? 0 }} 轮</span>
              <span>平均 {{ formatCompactNumber(averageTurnTokens) }}</span>
              <span>第 {{ latestTokenTurn?.index ?? 0 }} 轮</span>
            </div>
          </div>
        </div>

        <div class="usage-chart-card usage-chart-card--total">
          <div class="usage-chart-header">
            <div>
              <div class="usage-chart-kicker">总量趋势</div>
              <div class="usage-chart-title">累计 Token</div>
            </div>
            <div class="usage-chart-metrics">
              <div>
                <span>区间 Prompt</span>
                <strong>{{ formatCompactNumber(visibleInputTokens) }}</strong>
              </div>
              <div>
                <span>区间输出</span>
                <strong>{{ formatCompactNumber(visibleOutputTokens) }}</strong>
              </div>
            </div>
          </div>
          <div class="usage-chart-frame">
            <div ref="cumulativeChartRef" class="usage-echart" />
            <div class="usage-chart-caption">
              <span>第 {{ visibleTokenTurnPoints[0]?.index ?? 0 }} 轮</span>
              <span>总计 {{ formatCompactNumber(totalTokens) }}</span>
              <span>当前</span>
            </div>
          </div>
        </div>
      </div>

      <div v-else class="usage-chart-empty">还没有足够的 token 数据。</div>
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

.usage-trends-panel {
  margin-top: 16px;
  border: 1px solid #e5e7eb;
  border-radius: 20px;
  background:
    radial-gradient(circle at 12% 0%, rgba(148, 163, 184, 0.12), transparent 34%),
    linear-gradient(180deg, rgba(255, 255, 255, 0.96), rgba(248, 250, 252, 0.82));
  padding: 16px;
}

.dark .usage-trends-panel {
  border-color: #333;
  background:
    radial-gradient(circle at 12% 0%, rgba(148, 163, 184, 0.14), transparent 34%),
    linear-gradient(180deg, rgba(40, 40, 40, 0.96), rgba(31, 31, 31, 0.9));
}

.usage-trends-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 14px;
}

.usage-trends-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}

.usage-trends-title {
  color: #1a1a1a;
  font-size: 16px;
  font-weight: 680;
  letter-spacing: -0.01em;
}

.usage-trends-subtitle {
  margin-top: 4px;
  color: #8b816f;
  font-size: 12px;
}

.usage-trends-pill {
  flex-shrink: 0;
  border: 1px solid #e1d8c8;
  border-radius: 999px;
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

.dark .usage-trends-title {
  color: #ececec;
}

.dark .usage-trends-subtitle {
  color: #a99f90;
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

.usage-trend-grid {
  display: grid;
  gap: 12px;
}

@media (min-width: 1024px) {
  .usage-trend-grid {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }
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

.usage-chart-card::before {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  opacity: 0.55;
}

.usage-chart-card--turn::before {
  background: linear-gradient(135deg, rgba(177, 112, 52, 0.13), transparent 42%);
}

.usage-chart-card--total::before {
  background: linear-gradient(135deg, rgba(63, 127, 114, 0.13), transparent 42%);
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

.usage-chart-caption,
.usage-chart-empty {
  color: #8b816f;
  font-size: 11px;
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

.dark .usage-chart-metrics > div {
  border-color: rgba(70, 65, 58, 0.8);
  background: rgba(255, 255, 255, 0.04);
}

.dark .usage-chart-metrics span {
  color: #a99f90;
}

.dark .usage-chart-metrics strong {
  color: #ececec;
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

.usage-chart-caption {
  display: flex;
  justify-content: space-between;
  margin-top: -2px;
}

.usage-chart-empty {
  margin-top: 16px;
  border: 1px dashed #e2dacc;
  border-radius: 12px;
  padding: 18px;
  text-align: center;
}

.dark .usage-chart-subtitle,
.dark .usage-chart-caption,
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
