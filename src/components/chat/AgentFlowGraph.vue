<script setup lang="ts">
import { computed, ref } from 'vue';
import type { FlowNodeEntry, ToolExecutionEntry } from '../../lib/chat-types';

const props = defineProps<{
  entries: ToolExecutionEntry[];
  flowNodes: FlowNodeEntry[];
  isGenerating: boolean;
  hasMessages: boolean;
  lastUserMessage?: string;
  lastAssistantMessage?: string;
}>();

interface FlowNode {
  id: string;
  label: string;
  subLabel?: string;
  type: 'start' | 'llm' | 'tool' | 'end' | 'pipeline';
  status: 'idle' | 'running' | 'completed' | 'error' | 'cancelled' | 'skipped';
  pipelineDetail?: string;
}

const nodes = computed<FlowNode[]>(() => {
  if (!props.hasMessages) return [];

  const list: FlowNode[] = [];
  list.push({ id: 'start', label: '用户输入', type: 'start', status: 'completed' });

  for (const fn of props.flowNodes) {
    if (fn.nodeId === 'llm') continue;
    const truncated = fn.detail ? (fn.detail.length > 32 ? `${fn.detail.slice(0, 32)}…` : fn.detail) : undefined;
    list.push({
      id: `pipeline:${fn.nodeId}`,
      label: fn.label,
      subLabel: truncated,
      type: 'pipeline',
      status: fn.status,
      pipelineDetail: fn.detail,
    });
  }

  const llmEvent = props.flowNodes.find((n) => n.nodeId === 'llm');
  const llmSub = llmEvent?.detail
    ? (llmEvent.detail.length > 32 ? `${llmEvent.detail.slice(0, 32)}…` : llmEvent.detail)
    : undefined;
  list.push({
    id: 'llm',
    label: 'Nova 推理',
    subLabel: llmSub,
    type: 'llm',
    status: props.isGenerating && props.entries.length === 0 ? 'running' : 'completed',
  });

  for (const entry of props.entries) {
    list.push({
      id: entry.id,
      label: entry.toolName,
      subLabel: entry.status === 'running' ? '执行中…' : entry.status === 'error' ? '失败' : '完成',
      type: 'tool',
      status: entry.status,
    });
  }

  if (!props.isGenerating || props.entries.length > 0) {
    list.push({
      id: 'output',
      label: '输出响应',
      type: 'end',
      status: props.isGenerating ? 'running' : 'completed',
    });
  }

  return list;
});

const NODE_W = 200;
const NODE_H = 56;
const V_GAP = 48;
const CANVAS_W = 320;
const NODE_X = (CANVAS_W - NODE_W) / 2;

const nodeY = (index: number) => 20 + index * (NODE_H + V_GAP);

const connectorPath = (fromIndex: number): string => {
  const x = CANVAS_W / 2;
  const y1 = nodeY(fromIndex) + NODE_H;
  const y2 = nodeY(fromIndex + 1);
  const mid = (y1 + y2) / 2;
  return `M ${x} ${y1} C ${x} ${mid} ${x} ${mid} ${x} ${y2}`;
};

const nodeColors: Record<FlowNode['type'], { fill: string; stroke: string; text: string }> = {
  start: { fill: '#f0fdf4', stroke: '#86efac', text: '#16a34a' },
  llm: { fill: '#eff6ff', stroke: '#93c5fd', text: '#1d4ed8' },
  tool: { fill: '#faf5ff', stroke: '#d8b4fe', text: '#7c3aed' },
  end: { fill: '#fff7ed', stroke: '#fdba74', text: '#c2410c' },
  pipeline: { fill: '#f8fafc', stroke: '#cbd5e1', text: '#475569' },
};

const darkNodeColors: Record<FlowNode['type'], { fill: string; stroke: string; text: string }> = {
  start: { fill: '#052e16', stroke: '#166534', text: '#86efac' },
  llm: { fill: '#172554', stroke: '#1e40af', text: '#93c5fd' },
  tool: { fill: '#2e1065', stroke: '#6d28d9', text: '#d8b4fe' },
  end: { fill: '#431407', stroke: '#9a3412', text: '#fdba74' },
  pipeline: { fill: '#1a1f2e', stroke: '#334155', text: '#94a3b8' },
};

const statusDot: Record<string, string> = {
  idle: '#d1d5db',
  running: '#f59e0b',
  completed: '#22c55e',
  error: '#ef4444',
  cancelled: '#9ca3af',
  skipped: '#e5e7eb',
};

const selectedId = ref<string | null>(null);

interface MsgEntry {
  role: string;
  content: string;
  chars: number;
}

interface CompactDiff {
  type: 'compact_diff';
  summary: string;
  before: MsgEntry[];
  after: MsgEntry[];
}

interface FullTextModal {
  title: string;
  content: string;
}

const fullTextModal = ref<FullTextModal | null>(null);
const modalCopied = ref(false);
let modalCopyTimer: ReturnType<typeof setTimeout> | null = null;

const compactDiffData = computed<CompactDiff | null>(() => {
  if (!fullTextModal.value) return null;
  try {
    const parsed = JSON.parse(fullTextModal.value.content);
    if (parsed?.type === 'compact_diff') return parsed as CompactDiff;
  } catch {
    // Not JSON payload.
  }
  return null;
});

const compactDiffRemovedCount = computed(() => {
  const data = compactDiffData.value;
  if (!data) return 0;
  return data.before.filter((b) => !data.after.some((a) => a.content === b.content)).length;
});

const compactDiffAddedCount = computed(() => {
  const data = compactDiffData.value;
  if (!data) return 0;
  return data.after.filter((a) => !data.before.some((b) => b.content === a.content)).length;
});

function openFullText(title: string, content: string) {
  fullTextModal.value = { title, content };
  modalCopied.value = false;
}

function cardPreviewText(detail: string | undefined): string {
  if (!detail) return '无详情';
  try {
    const parsed = JSON.parse(detail);
    if (parsed?.type === 'compact_diff' && parsed.summary) return parsed.summary as string;
  } catch {
    // Not JSON payload.
  }
  return detail;
}

function closeFullText() {
  fullTextModal.value = null;
}

function copyModalText() {
  const text = fullTextModal.value?.content;
  if (!text) return;
  void navigator.clipboard.writeText(text).then(() => {
    modalCopied.value = true;
    if (modalCopyTimer) clearTimeout(modalCopyTimer);
    modalCopyTimer = setTimeout(() => {
      modalCopied.value = false;
    }, 2000);
  });
}

const selectedEntry = computed<ToolExecutionEntry | null>(() => {
  if (!selectedId.value || selectedId.value === 'start') return null;
  return props.entries.find((e) => e.id === selectedId.value) ?? null;
});

const isStartSelected = computed(() => selectedId.value === 'start');
const isOutputSelected = computed(() => selectedId.value === 'output');
const isLlmSelected = computed(() => selectedId.value === 'llm');
const selectedPipelineNode = computed<FlowNode | null>(() => {
  if (!selectedId.value?.startsWith('pipeline:')) return null;
  return nodes.value.find((n) => n.id === selectedId.value) ?? null;
});

const llmFlowNode = computed(() => props.flowNodes.find((n) => n.nodeId === 'llm') ?? null);

const toggleNode = (node: FlowNode) => {
  selectedId.value = selectedId.value === node.id ? null : node.id;
};

const formatTs = (ts: number) =>
  new Date(ts).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' });

const scale = ref(1);
const panX = ref(0);
const panY = ref(0);
const isDragging = ref(false);
const dragStart = ref({ x: 0, y: 0, panX: 0, panY: 0 });

const transformStr = computed(() => `translate(${panX.value}, ${panY.value}) scale(${scale.value})`);

const onWheel = (e: WheelEvent) => {
  e.preventDefault();
  const delta = -e.deltaY * 0.001;
  const newScale = Math.max(0.25, Math.min(4, scale.value + delta * scale.value));
  const svgEl = e.currentTarget as SVGElement;
  const rect = svgEl.getBoundingClientRect();
  const cx = e.clientX - rect.left;
  const cy = e.clientY - rect.top;
  const svgX = (cx - panX.value) / scale.value;
  const svgY = (cy - panY.value) / scale.value;
  panX.value = cx - svgX * newScale;
  panY.value = cy - svgY * newScale;
  scale.value = newScale;
};

const onMouseDown = (e: MouseEvent) => {
  if (e.button !== 0) return;
  isDragging.value = true;
  dragStart.value = { x: e.clientX, y: e.clientY, panX: panX.value, panY: panY.value };
};

const onMouseMove = (e: MouseEvent) => {
  if (!isDragging.value) return;
  panX.value = dragStart.value.panX + (e.clientX - dragStart.value.x);
  panY.value = dragStart.value.panY + (e.clientY - dragStart.value.y);
};

const stopDrag = () => {
  isDragging.value = false;
};

const resetView = () => {
  scale.value = 1;
  panX.value = 0;
  panY.value = 0;
};
</script>

<template>
  <div v-if="!hasMessages" class="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
    <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="opacity-30">
      <circle cx="12" cy="5" r="2"/><circle cx="5" cy="19" r="2"/><circle cx="19" cy="19" r="2"/>
      <line x1="12" y1="7" x2="5" y2="17"/><line x1="12" y1="7" x2="19" y2="17"/>
    </svg>
    <p class="text-sm">开始对话后将自动显示 Agent 流图</p>
  </div>

  <div v-else class="relative h-full min-h-0 w-full select-none overflow-hidden">
    <button
      class="absolute right-3 top-3 z-10 flex items-center gap-1.5 rounded-md border border-[#e7e2d7] bg-white/80 px-2.5 py-1.5 text-[11px] text-muted-foreground shadow-sm backdrop-blur-sm transition-colors hover:bg-white dark:border-[#333] dark:bg-[#252525]/80 dark:hover:bg-[#2e2e2e]"
      @click="resetView"
    >
      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
        <path d="M3 3v5h5"/>
      </svg>
      重置视图
    </button>

    <span class="pointer-events-none absolute bottom-3 right-3 z-10 text-[10px] tabular-nums text-muted-foreground/60">
      {{ Math.round(scale * 100) }}%
    </span>

    <svg
      class="h-full w-full"
      :style="{ cursor: isDragging ? 'grabbing' : 'grab' }"
      @wheel.prevent="onWheel"
      @mousedown="onMouseDown"
      @mousemove="onMouseMove"
      @mouseup="stopDrag"
      @mouseleave="stopDrag"
    >
      <g :transform="transformStr">
        <defs>
          <clipPath v-for="(node, i) in nodes" :key="`clip-${node.id}`" :id="`clip-${i}`">
            <rect :x="NODE_X + 14" :y="nodeY(i)" :width="NODE_W - 44" :height="NODE_H" />
          </clipPath>
        </defs>

        <path
          v-for="(_, i) in nodes.slice(0, -1)"
          :key="`conn-${i}`"
          :d="connectorPath(i)"
          fill="none"
          stroke="#d1d5db"
          stroke-width="1.5"
          stroke-dasharray="4 3"
          class="dark:stroke-[#444]"
        />

        <polygon
          v-for="(_, i) in nodes.slice(0, -1)"
          :key="`arrow-${i}`"
          :points="`${CANVAS_W / 2 - 5},${nodeY(i + 1) - 7} ${CANVAS_W / 2 + 5},${nodeY(i + 1) - 7} ${CANVAS_W / 2},${nodeY(i + 1) - 1}`"
          fill="#d1d5db"
          class="dark:fill-[#444]"
        />

        <g
          v-for="(node, i) in nodes"
          :key="node.id"
          :style="{ cursor: 'pointer' }"
          @click.stop="toggleNode(node)"
        >
          <rect
            v-if="selectedId === node.id"
            :x="NODE_X - 3"
            :y="nodeY(i) - 3"
            :width="NODE_W + 6"
            :height="NODE_H + 6"
            rx="13"
            :fill="nodeColors[node.type].stroke"
            opacity="0.4"
            class="dark:hidden"
          />
          <rect
            v-if="selectedId === node.id"
            :x="NODE_X - 3"
            :y="nodeY(i) - 3"
            :width="NODE_W + 6"
            :height="NODE_H + 6"
            rx="13"
            :fill="darkNodeColors[node.type].stroke"
            opacity="0.45"
            class="hidden dark:block"
          />
          <rect
            :x="NODE_X"
            :y="nodeY(i)"
            :width="NODE_W"
            :height="NODE_H"
            rx="10"
            :fill="nodeColors[node.type].fill"
            :stroke="nodeColors[node.type].stroke"
            stroke-width="1.5"
            class="dark:hidden"
          />
          <rect
            :x="NODE_X"
            :y="nodeY(i)"
            :width="NODE_W"
            :height="NODE_H"
            rx="10"
            :fill="darkNodeColors[node.type].fill"
            :stroke="darkNodeColors[node.type].stroke"
            stroke-width="1.5"
            class="hidden dark:block"
          />
          <circle :cx="NODE_X + NODE_W - 14" :cy="nodeY(i) + NODE_H / 2" r="5" :fill="statusDot[node.status]">
            <animate v-if="node.status === 'running'" attributeName="opacity" values="1;0.3;1" dur="1.2s" repeatCount="indefinite"/>
          </circle>
          <text
            :x="NODE_X + 16"
            :y="nodeY(i) + (node.subLabel ? NODE_H / 2 - 5 : NODE_H / 2 + 5)"
            font-size="13"
            font-weight="600"
            font-family="ui-sans-serif, system-ui, sans-serif"
            dominant-baseline="middle"
            :fill="nodeColors[node.type].text"
            :clip-path="`url(#clip-${i})`"
            class="dark:hidden"
          >{{ node.label }}</text>
          <text
            :x="NODE_X + 16"
            :y="nodeY(i) + (node.subLabel ? NODE_H / 2 - 5 : NODE_H / 2 + 5)"
            font-size="13"
            font-weight="600"
            font-family="ui-sans-serif, system-ui, sans-serif"
            dominant-baseline="middle"
            :fill="darkNodeColors[node.type].text"
            :clip-path="`url(#clip-${i})`"
            class="hidden dark:block"
          >{{ node.label }}</text>
          <text
            v-if="node.subLabel"
            :x="NODE_X + 16"
            :y="nodeY(i) + NODE_H / 2 + 10"
            font-size="11"
            font-family="ui-sans-serif, system-ui, sans-serif"
            dominant-baseline="middle"
            fill="#9ca3af"
            :clip-path="`url(#clip-${i})`"
          >{{ node.subLabel }}</text>
        </g>
      </g>
    </svg>

    <Transition name="detail-fade">
      <div
        v-if="selectedEntry || isStartSelected || isOutputSelected || selectedPipelineNode || isLlmSelected"
        class="absolute bottom-4 left-1/2 z-10 w-[90%] max-w-sm -translate-x-1/2 overflow-hidden rounded-xl border border-[#e7e2d7] bg-white/95 text-sm shadow-lg backdrop-blur-sm dark:border-[#333] dark:bg-[#252525]/95"
        @mousedown.stop
      >
        <template v-if="isStartSelected">
          <div class="flex items-center justify-between border-b border-[#e7e2d7] bg-[#f0fdf4] px-4 py-2.5 dark:border-[#333] dark:bg-[#052e16]">
            <span class="font-semibold text-[#16a34a] dark:text-[#86efac]">用户输入</span>
            <div class="flex items-center gap-2">
              <button v-if="lastUserMessage" class="text-[11px] text-[#9ca3af] transition-colors hover:text-[#16a34a] dark:hover:text-[#86efac]" @click.stop="openFullText('用户输入', lastUserMessage)">预览全文</button>
              <button class="text-[#9ca3af] transition-colors hover:text-[#1a1a1a] dark:hover:text-white" @click.stop="selectedId = null">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>
          <div class="px-4 py-3">
            <pre class="custom-scrollbar max-h-48 whitespace-pre-wrap break-all text-xs leading-relaxed text-[#1a1a1a] dark:text-[#ececec]">{{ lastUserMessage || '（无）' }}</pre>
          </div>
        </template>

        <template v-else-if="selectedEntry">
          <div class="flex items-center justify-between border-b border-[#e7e2d7] bg-[#f5f2ec] px-4 py-2.5 dark:border-[#333] dark:bg-[#2a2a2a]">
            <span class="truncate font-semibold text-[#7c3aed] dark:text-[#d8b4fe]">{{ selectedEntry.toolName }}</span>
            <div class="ml-2 flex shrink-0 items-center gap-2 text-[11px] text-[#9ca3af]">
              <span>{{ formatTs(selectedEntry.startedAt) }}</span>
              <button
                v-if="selectedEntry.input || selectedEntry.result"
                class="transition-colors hover:text-[#7c3aed] dark:hover:text-[#d8b4fe]"
                @click.stop="openFullText(selectedEntry.toolName, `【输入】\n${selectedEntry.input || '（无）'}\n\n【输出】\n${selectedEntry.result || '（执行中…）'}`)"
              >预览全文</button>
              <button class="transition-colors hover:text-[#1a1a1a] dark:hover:text-white" @click.stop="selectedId = null">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>
          <div class="border-b border-[#e7e2d7] px-4 py-3 dark:border-[#333]">
            <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-[#9ca3af]">输入</p>
            <pre class="custom-scrollbar max-h-28 overflow-auto whitespace-pre-wrap break-all text-xs leading-relaxed text-[#1a1a1a] dark:text-[#ececec]">{{ selectedEntry.input || '（无）' }}</pre>
          </div>
          <div class="px-4 py-3">
            <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-[#9ca3af]">输出</p>
            <pre class="custom-scrollbar max-h-32 overflow-auto whitespace-pre-wrap break-all text-xs leading-relaxed text-[#1a1a1a] dark:text-[#ececec]">{{ selectedEntry.result || (selectedEntry.status === 'running' ? '执行中…' : '（无）') }}</pre>
          </div>
        </template>

        <template v-else-if="isOutputSelected">
          <div class="flex items-center justify-between border-b border-[#e7e2d7] bg-[#fff7ed] px-4 py-2.5 dark:border-[#333] dark:bg-[#431407]">
            <span class="font-semibold text-[#c2410c] dark:text-[#fdba74]">输出响应</span>
            <div class="flex items-center gap-2">
              <button v-if="lastAssistantMessage" class="text-[11px] text-[#9ca3af] transition-colors hover:text-[#c2410c] dark:hover:text-[#fdba74]" @click.stop="openFullText('输出响应', lastAssistantMessage)">预览全文</button>
              <button class="text-[#9ca3af] transition-colors hover:text-[#1a1a1a] dark:hover:text-white" @click.stop="selectedId = null">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>
          <div class="px-4 py-3">
            <pre class="custom-scrollbar max-h-48 overflow-auto whitespace-pre-wrap break-all text-xs leading-relaxed text-[#1a1a1a] dark:text-[#ececec]">{{ lastAssistantMessage || '（响应尚未生成）' }}</pre>
          </div>
        </template>

        <template v-else-if="isLlmSelected">
          <div class="flex items-center justify-between border-b border-[#e7e2d7] bg-[#eff6ff] px-4 py-2.5 dark:border-[#333] dark:bg-[#172554]">
            <span class="font-semibold text-[#1d4ed8] dark:text-[#93c5fd]">Nova 推理</span>
            <div class="flex items-center gap-2">
              <button v-if="llmFlowNode?.detail" class="text-[11px] text-[#9ca3af] transition-colors hover:text-[#1d4ed8] dark:hover:text-[#93c5fd]" @click.stop="openFullText('Nova 推理', llmFlowNode.detail)">预览全文</button>
              <button class="text-[#9ca3af] transition-colors hover:text-[#1a1a1a] dark:hover:text-white" @click.stop="selectedId = null">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>
          <div class="px-4 py-3">
            <pre v-if="llmFlowNode?.detail" class="custom-scrollbar max-h-56 overflow-auto whitespace-pre-wrap break-all text-xs leading-relaxed text-[#1a1a1a] dark:text-[#ececec]">{{ llmFlowNode.detail }}</pre>
            <p v-else class="text-xs text-muted-foreground">（等待 LLM 请求…）</p>
          </div>
        </template>

        <template v-else-if="selectedPipelineNode">
          <div class="flex items-center justify-between border-b border-[#e7e2d7] bg-[#f8fafc] px-4 py-2.5 dark:border-[#333] dark:bg-[#1a1f2e]">
            <span class="font-semibold text-[#475569] dark:text-[#94a3b8]">{{ selectedPipelineNode.label }}</span>
            <div class="ml-2 flex shrink-0 items-center gap-2">
              <span :class="[
                'rounded-full px-1.5 py-0.5 text-[11px]',
                selectedPipelineNode.status === 'completed' ? 'bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-400' :
                selectedPipelineNode.status === 'running' ? 'bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400' :
                selectedPipelineNode.status === 'skipped' ? 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400' :
                'bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400',
              ]">{{ selectedPipelineNode.status }}</span>
              <button
                v-if="selectedPipelineNode.pipelineDetail"
                class="text-[11px] text-[#9ca3af] transition-colors hover:text-[#475569] dark:hover:text-[#94a3b8]"
                @click.stop="openFullText(selectedPipelineNode.label, selectedPipelineNode.pipelineDetail)"
              >预览全文</button>
              <button class="text-[#9ca3af] transition-colors hover:text-[#1a1a1a] dark:hover:text-white" @click.stop="selectedId = null">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>
          <div class="custom-scrollbar max-h-64 overflow-y-auto px-4 py-3">
            <p class="whitespace-pre-wrap break-words text-xs text-[#475569] dark:text-[#94a3b8]">{{ cardPreviewText(selectedPipelineNode.pipelineDetail) }}</p>
          </div>
        </template>
      </div>
    </Transition>

    <Transition name="modal-fade">
      <div
        v-if="fullTextModal"
        class="absolute inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px]"
        @mousedown.self="closeFullText"
      >
        <div
          class="relative flex flex-col overflow-hidden rounded-2xl border border-[#e7e2d7] bg-white shadow-2xl dark:border-[#444] dark:bg-[#1e1e1e]"
          :class="compactDiffData ? 'max-h-[88%] w-[96%] max-w-5xl' : 'max-h-[80%] w-[92%] max-w-2xl'"
          @mousedown.stop
        >
          <div class="flex shrink-0 items-center justify-between border-b border-[#e7e2d7] px-5 py-3 dark:border-[#333]">
            <span class="truncate text-sm font-semibold text-[#1a1a1a] dark:text-[#ececec]">{{ fullTextModal.title }}</span>
            <div class="ml-3 flex shrink-0 items-center gap-3">
              <button
                class="rounded-md border px-2.5 py-1 text-[12px] transition-colors"
                :class="modalCopied
                  ? 'border-green-300 text-green-600 dark:border-green-700 dark:text-green-400'
                  : 'border-[#e7e2d7] text-[#475569] hover:bg-black/5 dark:border-[#444] dark:text-[#94a3b8] dark:hover:bg-white/5'"
                @click="copyModalText"
              >{{ modalCopied ? '✓ 已复制' : '复制全文' }}</button>
              <button
                class="flex h-7 w-7 items-center justify-center rounded-md text-[#9ca3af] transition-colors hover:bg-black/5 hover:text-[#1a1a1a] dark:hover:bg-white/5 dark:hover:text-white"
                @click="closeFullText"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>

          <template v-if="compactDiffData">
            <div class="shrink-0 border-b border-[#e7e2d7] bg-[#f8fafc] px-5 py-2.5 dark:border-[#333] dark:bg-[#1a1f2e]">
              <pre class="whitespace-pre-wrap text-[11px] leading-relaxed text-[#475569] dark:text-[#94a3b8]">{{ compactDiffData.summary }}</pre>
            </div>
            <div class="flex min-h-0 flex-1 overflow-hidden">
              <div class="flex min-w-0 flex-1 flex-col border-r border-[#e7e2d7] dark:border-[#333]">
                <div class="flex shrink-0 items-center gap-2 border-b border-[#e7e2d7] bg-red-50 px-4 py-2 dark:border-[#333] dark:bg-red-950/30">
                  <span class="h-2 w-2 shrink-0 rounded-full bg-red-400"></span>
                  <span class="text-[11px] font-semibold text-red-600 dark:text-red-400">压缩前 — {{ compactDiffData.before.length }} 条</span>
                </div>
                <div class="custom-scrollbar flex-1 space-y-2 overflow-y-auto px-3 py-2">
                  <div
                    v-for="(msg, i) in compactDiffData.before"
                    :key="i"
                    class="rounded-lg border px-3 py-2"
                    :class="compactDiffData.after.some((a) => a.content === msg.content)
                      ? 'border-[#e7e2d7] bg-white dark:border-[#2a2a2a] dark:bg-[#252525]'
                      : 'border-red-200 bg-red-50/50 dark:border-red-900/50 dark:bg-red-950/20'"
                  >
                    <div class="mb-1 flex items-center gap-1.5">
                      <span
                        class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
                        :class="msg.role === '用户' ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300' : 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300'"
                      >{{ msg.role }}</span>
                      <span class="text-[10px] text-[#9ca3af]">{{ msg.chars }} 字符</span>
                      <span v-if="!compactDiffData.after.some((a) => a.content === msg.content)" class="ml-auto text-[10px] text-red-500 dark:text-red-400">已删除</span>
                    </div>
                    <pre class="custom-scrollbar max-h-40 overflow-y-auto whitespace-pre-wrap break-words text-[11px] leading-relaxed text-[#374151] dark:text-[#d1d5db]">{{ msg.content }}</pre>
                  </div>
                </div>
              </div>

              <div class="flex min-w-0 flex-1 flex-col">
                <div class="flex shrink-0 items-center gap-2 border-b border-[#e7e2d7] bg-green-50 px-4 py-2 dark:border-[#333] dark:bg-green-950/30">
                  <span class="h-2 w-2 shrink-0 rounded-full bg-green-400"></span>
                  <span class="text-[11px] font-semibold text-green-600 dark:text-green-400">压缩后 — {{ compactDiffData.after.length }} 条</span>
                </div>
                <div class="custom-scrollbar flex-1 space-y-2 overflow-y-auto px-3 py-2">
                  <div
                    v-for="(msg, i) in compactDiffData.after"
                    :key="i"
                    class="rounded-lg border px-3 py-2"
                    :class="compactDiffData.before.some((b) => b.content === msg.content)
                      ? 'border-[#e7e2d7] bg-white dark:border-[#2a2a2a] dark:bg-[#252525]'
                      : 'border-green-200 bg-green-50/50 dark:border-green-900/50 dark:bg-green-950/20'"
                  >
                    <div class="mb-1 flex items-center gap-1.5">
                      <span
                        class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
                        :class="msg.role === '用户' ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300' : 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300'"
                      >{{ msg.role }}</span>
                      <span class="text-[10px] text-[#9ca3af]">{{ msg.chars }} 字符</span>
                      <span v-if="!compactDiffData.before.some((b) => b.content === msg.content)" class="ml-auto text-[10px] text-green-500 dark:text-green-400">新增</span>
                    </div>
                    <pre class="custom-scrollbar max-h-40 overflow-y-auto whitespace-pre-wrap break-words text-[11px] leading-relaxed text-[#374151] dark:text-[#d1d5db]">{{ msg.content }}</pre>
                  </div>
                </div>
              </div>
            </div>
            <div class="flex shrink-0 gap-4 border-t border-[#e7e2d7] px-5 py-2 text-[10px] text-[#9ca3af] dark:border-[#333]">
              <span class="text-red-400">− {{ compactDiffData.before.length }} 条（压缩前）</span>
              <span class="text-green-400">+ {{ compactDiffData.after.length }} 条（压缩后）</span>
              <span class="ml-auto">删除 {{ compactDiffRemovedCount }} 条 · 新增 {{ compactDiffAddedCount }} 条</span>
            </div>
          </template>

          <template v-else>
            <div class="custom-scrollbar flex-1 overflow-y-auto px-5 py-4">
              <pre class="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-[#1a1a1a] dark:text-[#ececec]">{{ fullTextModal.content }}</pre>
            </div>
            <div class="flex shrink-0 gap-3 border-t border-[#e7e2d7] px-5 py-2 text-[10px] text-[#9ca3af] dark:border-[#333]">
              <span>{{ fullTextModal.content.length.toLocaleString() }} 字符</span>
              <span>{{ fullTextModal.content.split('\n').length.toLocaleString() }} 行</span>
            </div>
          </template>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.custom-scrollbar::-webkit-scrollbar {
  width: 5px;
}

.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}

.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: #e5e5e5;
  border-radius: 10px;
}

.dark .custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: #444;
}

.detail-fade-enter-active,
.detail-fade-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.detail-fade-enter-from,
.detail-fade-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(10px);
}

.detail-fade-enter-to,
.detail-fade-leave-from {
  opacity: 1;
  transform: translateX(-50%) translateY(0);
}

.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity 0.18s ease;
}

.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}
</style>
