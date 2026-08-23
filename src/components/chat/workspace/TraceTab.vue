<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import type { TurnTrace, WireCallTrace } from '../../../lib/chat-types';
import { getConversationTurnTraces } from '../../../features/chat/services/chat-api';
import JsonTree from './JsonTree.vue';

const props = defineProps<{
  conversationId: string | null;
}>();

const traces = ref<TurnTrace[]>([]);
const loading = ref(false);
const errorText = ref('');
/** 展开的回合 seq 集合（展开后展示发送/接收两块内容）。 */
const expandedSeqs = ref<Set<number>>(new Set());

const loadTraces = async () => {
  const target = props.conversationId;
  if (!target) {
    traces.value = [];
    return;
  }
  loading.value = true;
  errorText.value = '';
  try {
    const result = await getConversationTurnTraces(target);
    // 会话可能已切走，避免旧请求覆盖新会话数据。
    if (target !== props.conversationId) return;
    traces.value = result;
  } catch (err) {
    if (target === props.conversationId) {
      errorText.value = err instanceof Error ? err.message : String(err);
      traces.value = [];
    }
  } finally {
    if (target === props.conversationId) {
      loading.value = false;
    }
  }
};

watch(
  () => props.conversationId,
  () => {
    traces.value = [];
    expandedSeqs.value = new Set();
    void loadTraces();
  },
  { immediate: true },
);

const toggleTurn = (seq: number) => {
  const next = new Set(expandedSeqs.value);
  if (next.has(seq)) {
    next.delete(seq);
  } else {
    next.add(seq);
  }
  expandedSeqs.value = next;
};

const displayTurns = computed(() => [...traces.value].reverse());

/** 回合终态标签。 */
const turnStatusLabel = (turn: TurnTrace): string => {
  if (!turn.endedAt) return '进行中';
  const reason = turn.stopReason ?? '';
  if (['end_turn', 'completed', 'responses.completed'].includes(reason)) return '已完成';
  if (reason === 'cancelled') return '已取消';
  return reason || '已完成';
};

/** 回合最后一次发给模型的请求（工具循环多轮时最后一次即完整上下文）。 */
const lastWireCall = (turn: TurnTrace): WireCallTrace | null =>
  turn.wireCalls.length > 0 ? turn.wireCalls[turn.wireCalls.length - 1] : null;

const copyText = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // 剪贴板不可用时静默忽略。
  }
};

const prettyJson = (value: unknown) => JSON.stringify(value, null, 2);

/** wire 请求报文统一转成可读 JSON 文本。 */
const wireRequestText = (request: Record<string, unknown> | string) =>
  typeof request === 'string' ? request : prettyJson(request);
</script>

<template>
  <div class="flex h-full flex-col bg-white dark:bg-[#1e1e1e]">
    <!-- 头部：标题 + 刷新 -->
    <div class="flex shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 py-2 dark:border-[#333]">
      <div class="text-[13px] font-semibold text-[#111827] dark:text-[#ececec]">
        聊天轨迹
        <span class="ml-1 text-[11px] font-normal text-[#8a94a3] dark:text-[#858585]">{{ traces.length }} 个回合</span>
      </div>
      <Button variant="ghost" size="sm" class="h-7 gap-1 px-2 text-[12px]" :disabled="loading || !conversationId" @click="loadTraces">
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" :class="loading ? 'animate-spin' : ''">
          <path d="M21 12a9 9 0 1 1-2.64-6.36" />
          <path d="M21 3v6h-6" />
        </svg>
        {{ loading ? '加载中…' : '刷新' }}
      </Button>
    </div>

    <!-- 内容 -->
    <div class="min-h-0 flex-1 overflow-y-auto p-2">
      <p v-if="!conversationId" class="px-2 py-6 text-center text-[12px] text-[#8a94a3] dark:text-[#858585]">
        请先选择或创建一个会话
      </p>
      <p v-else-if="errorText" class="px-2 py-6 text-center text-[12px] text-red-500">加载失败：{{ errorText }}</p>
      <p v-else-if="!loading && traces.length === 0" class="px-2 py-6 text-center text-[12px] text-[#8a94a3] dark:text-[#858585]">
        该会话还没有交互记录
      </p>

      <div v-else class="flex flex-col gap-2">
        <div
          v-for="(turn, displayIndex) in displayTurns"
          :key="turn.seq"
          class="overflow-hidden rounded-lg border border-[#e5e7eb] bg-white dark:border-[#3a3a3a] dark:bg-[#242424]"
        >
          <!-- 回合头：Turn N · 状态，点击展开发送/接收 -->
          <button
            type="button"
            class="flex w-full items-center gap-1.5 px-3 py-2.5 text-left"
            @click="toggleTurn(turn.seq)"
          >
            <span class="shrink-0 text-[12px] font-semibold text-[#111827] dark:text-[#ececec]">
              Turn {{ traces.length - displayIndex }}
            </span>
            <span class="min-w-0 flex-1 truncate text-[12px] text-[#8a94a3] dark:text-[#858585]">
              · {{ turnStatusLabel(turn) }}
            </span>
            <svg
              width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
              stroke-linecap="round" stroke-linejoin="round"
              class="shrink-0 text-[#94a3b8] transition-transform"
              :class="expandedSeqs.has(turn.seq) ? 'rotate-180' : ''"
            >
              <path d="M6 9l6 6 6-6" />
            </svg>
          </button>

          <!-- 展开：仅发送 + 接收两块 -->
          <div v-if="expandedSeqs.has(turn.seq)" class="flex flex-col gap-2.5 border-t border-[#e5e7eb] px-3 py-2.5 dark:border-[#3a3a3a]">
            <template v-if="lastWireCall(turn)">
              <!-- 发送：最后一次发给模型的完整请求 -->
              <div>
                <div class="mb-1 flex items-center gap-1.5">
                  <span class="text-[10px] font-semibold text-[#0369a1] dark:text-sky-300">发送</span>
                  <button type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(wireRequestText(lastWireCall(turn)!.request))">复制</button>
                </div>
                <div class="max-h-72 overflow-auto rounded bg-[#f3f4f6] p-1.5 dark:bg-[#2a2a2a]">
                  <JsonTree :data="lastWireCall(turn)!.request" :default-expand-depth="1" />
                </div>
              </div>
              <!-- 接收：模型完整响应 JSON -->
              <div>
                <div class="mb-1 flex items-center gap-1.5">
                  <span class="text-[10px] font-semibold text-[#047857] dark:text-emerald-300">接收</span>
                  <button v-if="lastWireCall(turn)!.response" type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(wireRequestText(lastWireCall(turn)!.response!))">复制</button>
                </div>
                <div v-if="lastWireCall(turn)!.response" class="max-h-72 overflow-auto rounded bg-[#f3f4f6] p-1.5 dark:bg-[#2a2a2a]">
                  <JsonTree :data="lastWireCall(turn)!.response" :default-expand-depth="1" />
                </div>
                <div v-else class="rounded bg-[#f3f4f6] p-1.5 font-mono text-[10px] text-[#8a94a3] dark:bg-[#2a2a2a] dark:text-[#858585]">（无：取消/出错）</div>
              </div>
            </template>
            <p v-else class="text-[11px] text-[#8a94a3] dark:text-[#858585]">本回合没有模型请求记录。</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
