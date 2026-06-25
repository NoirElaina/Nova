<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
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

echarts.use([LineChart, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer]);

interface HeatmapPoint {
  date: string;
  tokens: number;
  sessions: number;
}

interface ModelBreakdown {
  model: string;
  tokens: number;
  calls: number;
  costUsd: string;
}

interface UsageStats {
  totalSessions: number;
  totalMessages: number;
  totalTokens: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCostUsd: string;
  activeDays: number;
  currentStreak: number;
  peakHour: number | null;
  favoriteModel: string | null;
  heatmap: HeatmapPoint[];
  modelBreakdown: ModelBreakdown[];
}

interface TokenUsageRecord {
  id: number;
  conversationId: string | null;
  model: string;
  provider: string | null;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalTokens: number;
  costUsd: string | null;
  source: string | null;
  createdAt: number;
}

const RECORD_LIMIT = 500;

const stats = ref<UsageStats | null>(null);
const records = ref<TokenUsageRecord[]>([]);
const loading = ref(false);
const loadError = ref<string | null>(null);

type TokenRange = 1 | 7 | 14 | 30 | 'all';
const selectedTokenRange = ref<TokenRange>('all');
const tokenRangeOptions: { label: string; value: TokenRange }[] = [
  { label: '今天', value: 1 },
  { label: '7 天', value: 7 },
  { label: '14 天', value: 14 },
  { label: '30 天', value: 30 },
  { label: '全部', value: 'all' },
];

const loadAll = async () => {
  loading.value = true;
  loadError.value = null;
  try {
    const [s, r] = await Promise.all([
      invoke<UsageStats>('get_usage_stats'),
      invoke<TokenUsageRecord[]>('list_token_usage', { limit: RECORD_LIMIT }),
    ]);
    stats.value = s;
    records.value = r;
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : String(error);
  } finally {
    loading.value = false;
  }
};

const filteredRecords = computed<TokenUsageRecord[]>(() => {
  if (selectedTokenRange.value === 'all') return records.value;
  const cutoff = Date.now() - selectedTokenRange.value * 24 * 60 * 60 * 1000;
  return records.value.filter((r) => r.createdAt * 1000 >= cutoff);
});

const perTurnValues = computed(() => filteredRecords.value.map((r) => r.totalTokens));
const perTurnMax = computed(() => Math.max(...perTurnValues.value, 1));
const latestRecord = computed(() => filteredRecords.value[filteredRecords.value.length - 1] ?? null);
const averageTurnTokens = computed(() =>
  filteredRecords.value.length > 0
    ? Math.round(perTurnValues.value.reduce((sum, v) => sum + v, 0) / filteredRecords.value.length)
    : 0,
);

const inputTokenValues = computed(() => filteredRecords.value.map((r) => r.inputTokens));
const outputTokenValues = computed(() => filteredRecords.value.map((r) => r.outputTokens));
const cacheCreationValues = computed(() => filteredRecords.value.map((r) => r.cacheCreationTokens));
const cacheReadValues = computed(() => filteredRecords.value.map((r) => r.cacheReadTokens));

const costValues = computed(() => {
  let cumulative = 0;
  return filteredRecords.value.map((r) => {
    const cost = parseFloat(r.costUsd ?? '0');
    if (!isNaN(cost)) cumulative += cost;
    return cumulative;
  });
});

const turnLabels = computed(() =>
  filteredRecords.value.map((r) => {
    const d = new Date(r.createdAt * 1000);
    const mm = String(d.getMonth() + 1).padStart(2, '0');
    const dd = String(d.getDate()).padStart(2, '0');
    const hh = String(d.getHours()).padStart(2, '0');
    const mi = String(d.getMinutes()).padStart(2, '0');
    return `${mm}-${dd} ${hh}:${mi}`;
  }),
);

type UsageChartOption = EChartsCoreOption & GridComponentOption & TooltipComponentOption;

const buildTrendOption = (): UsageChartOption => {
  const labels = turnLabels.value;
  return {
    animation: true,
    animationDuration: 600,
    animationEasing: 'cubicOut',
    grid: { left: 10, right: 50, top: 20, bottom: 62, containLabel: false },
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

const trendChartRef = ref<HTMLElement | null>(null);
let trendChart: ECharts | null = null;

const renderCharts = async () => {
  await nextTick();
  trendChart?.dispose();
  trendChart = null;
  if (!trendChartRef.value || filteredRecords.value.length === 0) return;
  trendChart = echarts.init(trendChartRef.value);
  trendChart.setOption(buildTrendOption(), true);
  trendChart.resize();
};

const resizeCharts = () => {
  trendChart?.resize();
};

watch(
  () => [selectedTokenRange.value, filteredRecords.value.length],
  () => {
    void renderCharts();
  },
  { immediate: true },
);

// Heatmap grid (GitHub-style calendar): week columns x 7 day rows.
type HeatmapCell = { date: string; tokens: number; count: number };
const heatmapGrid = computed<HeatmapCell[][]>(() => {
  const map = new Map(stats.value?.heatmap.map((h) => [h.date, h.tokens]) ?? []);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const start = new Date(today);
  start.setDate(start.getDate() - 364);
  // Align start to Sunday.
  start.setDate(start.getDate() - start.getDay());

  const weeks: HeatmapCell[][] = [];
  let week: HeatmapCell[] = [];
  const cursor = new Date(start);
  while (cursor <= today) {
    const key = formatDayKey(cursor);
    const tokens = map.get(key) ?? 0;
    const count = clampBucket(tokens);
    const cell: HeatmapCell = { date: key, tokens, count };
    if (cursor < today && cursor.getDay() === 0 && week.length > 0) {
      weeks.push(week);
      week = [];
    }
    week.push(cell);
    cursor.setDate(cursor.getDate() + 1);
  }
  if (week.length > 0) weeks.push(week);
  return weeks;
});

const formatDayKey = (d: Date) => {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
};

const clampBucket = (tokens: number) => {
  if (tokens <= 0) return 0;
  if (tokens < 1000) return 1;
  if (tokens < 10000) return 2;
  if (tokens < 100000) return 3;
  return 4;
};

const heatCellClass = (count: number) => `heat-cell lv-${count}`;

const sortedBreakdown = computed(() =>
  [...(stats.value?.modelBreakdown ?? [])].sort((a, b) => b.tokens - a.tokens),
);

const formatNumber = (value: number) => value.toLocaleString('en-US');
const formatCompactNumber = (value: number) => {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 10_000) return `${Math.round(value / 1_000)}k`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return `${value}`;
};
const formatCost = (value: number | string) => {
  const num = typeof value === 'string' ? parseFloat(value) : value;
  if (isNaN(num) || num <= 0) return '$0.00';
  if (num < 0.01) return '$' + num.toFixed(4);
  return '$' + num.toFixed(2);
};
const formatHour = (h: number | null) => (h === null ? '—' : `${String(h).padStart(2, '0')}:00`);
const formatModel = (m: string | null) => {
  if (!m) return '—';
  return m.length > 28 ? m.slice(0, 26) + '…' : m;
};

onMounted(() => {
  window.addEventListener('resize', resizeCharts);
  void loadAll().then(() => renderCharts());
});

onBeforeUnmount(() => {
  window.removeEventListener('resize', resizeCharts);
  trendChart?.dispose();
  trendChart = null;
});
</script>

<template>
  <div class="h-full overflow-y-auto px-4 py-4">
    <div v-if="loadError" class="usage-error">{{ loadError }}</div>

    <!-- Trend chart -->
    <div class="usage-chart-card mb-4">
      <div class="usage-chart-header">
        <div class="usage-chart-metrics-new">
          <div class="usage-metric-block">
            <span class="usage-metric-label">最新</span>
            <strong class="usage-metric-value">{{ formatCompactNumber(latestRecord?.totalTokens ?? 0) }}</strong>
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
          <div class="usage-trends-pill">{{ filteredRecords.length }}/{{ records.length }} 次</div>
        </div>
      </div>
      <div class="usage-chart-frame">
        <div v-if="filteredRecords.length > 0" ref="trendChartRef" class="usage-echart usage-echart--large" />
        <div v-else class="usage-chart-empty">还没有足够的 token 数据。</div>
      </div>
    </div>

    <!-- Overview stat cards -->
    <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <div class="usage-card">
        <div class="usage-label">总 Tokens</div>
        <div class="usage-value">{{ formatNumber(stats?.totalTokens ?? 0) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">Prompt Tokens</div>
        <div class="usage-value">{{ formatNumber(stats?.totalInputTokens ?? 0) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">输出 Tokens</div>
        <div class="usage-value">{{ formatNumber(stats?.totalOutputTokens ?? 0) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">总费用</div>
        <div class="usage-value">{{ formatCost(stats?.totalCostUsd ?? '0') }}</div>
      </div>
    </div>

    <!-- Activity stat cards -->
    <div class="mt-4 grid grid-cols-2 gap-3 lg:grid-cols-4">
      <div class="usage-card">
        <div class="usage-label">会话数</div>
        <div class="usage-value">{{ formatNumber(stats?.totalSessions ?? 0) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">消息数</div>
        <div class="usage-value">{{ formatNumber(stats?.totalMessages ?? 0) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">活跃天数</div>
        <div class="usage-value">{{ stats?.activeDays ?? 0 }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">连续天数</div>
        <div class="usage-value">{{ stats?.currentStreak ?? 0 }}</div>
      </div>
    </div>

    <div class="mt-4 grid grid-cols-2 gap-3 lg:grid-cols-3">
      <div class="usage-card">
        <div class="usage-label">峰值时段</div>
        <div class="usage-value">{{ formatHour(stats?.peakHour ?? null) }}</div>
      </div>
      <div class="usage-card">
        <div class="usage-label">常用模型</div>
        <div class="usage-value usage-value--sm">{{ formatModel(stats?.favoriteModel ?? null) }}</div>
      </div>
      <div class="usage-card usage-card--action">
        <button type="button" class="usage-refresh" :disabled="loading" @click="loadAll">
          {{ loading ? '加载中…' : '刷新' }}
        </button>
      </div>
    </div>

    <!-- Heatmap -->
    <div class="mt-4 usage-card">
      <div class="usage-card-head">
        <div class="usage-label">每日用量热力图</div>
        <div class="heat-legend">
          <span class="heat-legend-text">少</span>
          <span class="heat-cell lv-0" />
          <span class="heat-cell lv-1" />
          <span class="heat-cell lv-2" />
          <span class="heat-cell lv-3" />
          <span class="heat-cell lv-4" />
          <span class="heat-legend-text">多</span>
        </div>
      </div>
      <div class="heat-grid">
        <div v-for="(week, wi) in heatmapGrid" :key="wi" class="heat-week">
          <div
            v-for="cell in week"
            :key="cell.date"
            :class="heatCellClass(cell.count)"
            :title="`${cell.date} · ${formatNumber(cell.tokens)} tokens`"
          />
        </div>
      </div>
    </div>

    <!-- Model breakdown -->
    <div class="mt-4 usage-card">
      <div class="usage-label">按模型统计</div>
      <div v-if="sortedBreakdown.length > 0" class="usage-table">
        <div class="usage-table-row usage-table-row--head">
          <div>模型</div>
          <div class="text-right">Tokens</div>
          <div class="text-right">调用</div>
          <div class="text-right">费用</div>
        </div>
        <div v-for="row in sortedBreakdown" :key="row.model" class="usage-table-row">
          <div class="usage-table-model" :title="row.model">{{ row.model }}</div>
          <div class="text-right">{{ formatNumber(row.tokens) }}</div>
          <div class="text-right">{{ formatNumber(row.calls) }}</div>
          <div class="text-right">{{ formatCost(row.costUsd) }}</div>
        </div>
      </div>
      <div v-else class="usage-chart-empty">还没有可展示的用量数据。</div>
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

.usage-card--action {
  display: flex;
  align-items: center;
  justify-content: center;
}

.usage-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
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

.usage-value--sm {
  font-size: 15px;
  word-break: break-all;
}

.dark .usage-value {
  color: #ececec;
}

.usage-error {
  margin-bottom: 12px;
  border-radius: 12px;
  padding: 10px 12px;
  background: rgba(244, 63, 94, 0.08);
  color: #be123c;
  font-size: 13px;
}

.usage-refresh {
  border: 1px solid #e1d8c8;
  border-radius: 999px;
  background: transparent;
  color: #4f473c;
  cursor: pointer;
  font-size: 13px;
  padding: 8px 16px;
  transition: background 160ms ease, color 160ms ease;
}

.usage-refresh:hover:not(:disabled) {
  background: rgba(235, 228, 216, 0.72);
}

.usage-refresh:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.dark .usage-refresh {
  border-color: #46413a;
  color: #b9b0a2;
}

.dark .usage-refresh:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.07);
  color: #eee4d6;
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
  transition: background 160ms ease, color 160ms ease, box-shadow 160ms ease;
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

.usage-chart-empty {
  color: #8b816f;
  font-size: 11px;
  margin-top: 16px;
  border: 1px dashed #e2dacc;
  border-radius: 12px;
  padding: 18px;
  text-align: center;
}

.dark .usage-chart-empty {
  color: #a99f90;
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

/* Heatmap */
.heat-grid {
  display: flex;
  gap: 3px;
  overflow-x: auto;
  padding-bottom: 4px;
}

.heat-week {
  display: grid;
  grid-template-rows: repeat(7, 1fr);
  gap: 3px;
}

.heat-cell {
  width: 11px;
  height: 11px;
  border-radius: 2px;
  background: #ebedf0;
}

.dark .heat-cell {
  background: #2d2d2d;
}

.heat-cell.lv-1 { background: #c6e9c0; }
.heat-cell.lv-2 { background: #7bc77b; }
.heat-cell.lv-3 { background: #2a9d3f; }
.heat-cell.lv-4 { background: #167e2c; }

.dark .heat-cell.lv-1 { background: #0e4429; }
.dark .heat-cell.lv-2 { background: #006d32; }
.dark .heat-cell.lv-3 { background: #26a641; }
.dark .heat-cell.lv-4 { background: #39d353; }

.heat-legend {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.heat-legend-text {
  color: #8b816f;
  font-size: 10px;
  margin: 0 2px;
}

/* Model breakdown table */
.usage-table {
  margin-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.usage-table-row {
  display: grid;
  grid-template-columns: 1fr 80px 70px 90px;
  gap: 8px;
  padding: 8px 4px;
  font-size: 13px;
  color: #1a1a1a;
  border-bottom: 1px solid rgba(226, 216, 201, 0.5);
}

.usage-table-row:last-child {
  border-bottom: 0;
}

.usage-table-row--head {
  color: #8b816f;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  border-bottom: 1px solid rgba(226, 216, 201, 0.75);
}

.usage-table-model {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dark .usage-table-row {
  color: #ececec;
  border-bottom-color: rgba(67, 62, 55, 0.5);
}

.dark .usage-table-row--head {
  color: #a99f90;
  border-bottom-color: rgba(67, 62, 55, 0.75);
}
</style>
