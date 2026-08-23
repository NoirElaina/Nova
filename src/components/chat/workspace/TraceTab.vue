<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import type { TurnTrace } from '../../../lib/chat-types';
import { getConversationTurnTraces } from '../../../features/chat/services/chat-api';
import JsonTree from './JsonTree.vue';

const props = defineProps<{
  conversationId: string | null;
}>();

const traces = ref<TurnTrace[]>([]);
const loading = ref(false);
const errorText = ref('');
/** 展开的回合 seq 集合。 */
const expandedSeqs = ref<Set<number>>(new Set());
/** 展开的工具调用 key（turnSeq:callId）。 */
const expandedTools = ref<Set<string>>(new Set());
/** 展开的模型 API 请求 key（turnSeq:index）。 */
const expandedWire = ref<Set<string>>(new Set());

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
    expandedTools.value = new Set();
    expandedWire.value = new Set();
    void loadTraces();
  },
  { immediate: true },
);

const toggleSet = (set: Set<number> | Set<string>, key: number | string) => {
  const next = new Set(set as Set<number & string>);
  if (next.has(key as number & string)) {
    next.delete(key as number & string);
  } else {
    next.add(key as number & string);
  }
  if (set === expandedSeqs.value) {
    expandedSeqs.value = next as Set<number>;
  } else if (set === expandedTools.value) {
    expandedTools.value = next as Set<string>;
  } else {
    expandedWire.value = next as Set<string>;
  }
};

const toggleTurn = (seq: number) => toggleSet(expandedSeqs.value, seq);
const toggleTool = (key: string) => toggleSet(expandedTools.value, key);
const toggleWire = (key: string) => toggleSet(expandedWire.value, key);

const displayTurns = computed(() => [...traces.value].reverse());

const formatTime = (ms?: number | null) => {
  if (!ms || ms <= 0) return '';
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return '';
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
};

/** 回合耗时（毫秒），未结束返回空。 */
const turnDurationMs = (turn: TurnTrace) => {
  if (!turn.endedAt || turn.endedAt <= turn.startedAt) return null;
  return turn.endedAt - turn.startedAt;
};

const formatDuration = (ms?: number | null) => {
  if (!ms || ms <= 0) return '';
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
};

const toolDuration = (startedAt: number, finishedAt?: number | null) => {
  if (!finishedAt || finishedAt <= startedAt) return '';
  return formatDuration(finishedAt - startedAt);
};

const copyText = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // 剪贴板不可用时静默忽略。
  }
};

/** 大文本折叠预览。 */
const previewText = (text: string, limit = 600) => {
  if (text.length <= limit) return text;
  return `${text.slice(0, limit)}\n…（共 ${text.length.toLocaleString()} 字符）`;
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
      <div class="min-w-0">
        <div class="text-[13px] font-semibold text-[#111827] dark:text-[#ececec]">聊天轨迹</div>
        <div class="text-[11px] text-[#8a94a3] dark:text-[#858585]">
          每个回合的完整交互记录（事件日志投影）· {{ traces.length }} 个回合
        </div>
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
          class="rounded-lg border border-[#e5e7eb] bg-[#fafafa] dark:border-[#3a3a3a] dark:bg-[#242424]"
        >
          <!-- 回合头：点击展开 -->
          <button
            type="button"
            class="flex w-full items-center gap-2 px-2.5 py-2 text-left"
            @click="toggleTurn(turn.seq)"
          >
            <svg
              width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
              stroke-linecap="round" stroke-linejoin="round"
              class="shrink-0 text-[#64748b] transition-transform dark:text-[#9ca3af]"
              :class="expandedSeqs.has(turn.seq) ? 'rotate-90' : ''"
            >
              <path d="M9 6l6 6-6 6" />
            </svg>
            <span class="shrink-0 text-[12px] font-semibold text-[#111827] dark:text-[#ececec]">
              #{{ traces.length - displayIndex }}
            </span>
            <span class="shrink-0 text-[11px] text-[#8a94a3] dark:text-[#858585]">{{ formatTime(turn.startedAt) }}</span>
            <span
              v-if="turn.stopReason"
              class="shrink-0 rounded px-1 py-px font-mono text-[10px]"
              :class="turn.stopReason === 'end_turn'
                ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300'
                : 'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-300'"
            >{{ turn.stopReason }}</span>
            <span v-if="turn.toolCalls.length > 0" class="shrink-0 text-[11px] text-[#64748b] dark:text-[#9ca3af]">
              {{ turn.toolCalls.length }} 工具
            </span>
            <span v-if="turnDurationMs(turn)" class="shrink-0 text-[11px] text-[#64748b] dark:text-[#9ca3af]">
              {{ formatDuration(turnDurationMs(turn)) }}
            </span>
            <span v-if="turn.tokenUsage" class="shrink-0 font-mono text-[11px] text-[#047857] dark:text-[#86efac]">
              {{ turn.tokenUsage.toLocaleString() }} tok
            </span>
            <span class="min-w-0 flex-1 truncate text-[11px] text-[#8a94a3] dark:text-[#858585]">
              {{ turn.userInput || turn.assistantText || '' }}
            </span>
          </button>

          <!-- 回合详情 -->
          <div v-if="expandedSeqs.has(turn.seq)" class="flex flex-col gap-2 border-t border-[#e5e7eb] px-2.5 py-2 dark:border-[#3a3a3a]">
            <!-- 用户输入 -->
            <section v-if="turn.userInput">
              <div class="mb-1 flex items-center gap-1.5">
                <span class="text-[11px] font-semibold text-[#334155] dark:text-[#ccc]">用户输入</span>
                <button type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(turn.userInput!)">复制</button>
              </div>
              <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-[#e5e7eb] bg-white p-2 font-mono text-[11px] leading-relaxed text-[#111827] dark:border-[#3a3a3a] dark:bg-[#1b1b1b] dark:text-[#e5e5e5]">{{ previewText(turn.userInput) }}</pre>
            </section>

            <!-- 注入上下文统计 -->
            <p v-if="turn.injectedContextCount > 0" class="text-[11px] text-[#8a94a3] dark:text-[#858585]">
              另有 {{ turn.injectedContextCount }} 条注入上下文（hook/压缩注入，不进 UI 历史）
            </p>

            <!-- 压缩事件 -->
            <div v-for="(compact, compactIndex) in turn.compactions" :key="compactIndex" class="flex items-center gap-1.5 rounded-md bg-violet-50 px-2 py-1 text-[11px] text-violet-700 dark:bg-violet-500/10 dark:text-violet-300">
              <span>压缩 [{{ compact.level }}]</span>
              <span class="font-mono">{{ compact.tokensBefore.toLocaleString() }} → {{ compact.tokensAfter.toLocaleString() }} tok</span>
            </div>

            <!-- 模型 API 请求：每条 = 一次发送 + 一次接收 -->
            <section v-if="turn.wireCalls.length > 0">
              <div class="mb-1 text-[11px] font-semibold text-[#334155] dark:text-[#ccc]">
                模型请求（{{ turn.wireCalls.length }}）
              </div>
              <div class="flex flex-col gap-1">
                <div
                  v-for="(call, wireIndex) in turn.wireCalls"
                  :key="wireIndex"
                  class="rounded-md border border-[#e5e7eb] bg-white dark:border-[#3a3a3a] dark:bg-[#1b1b1b]"
                >
                  <button
                    type="button"
                    class="flex w-full items-center gap-1.5 px-2 py-1.5 text-left"
                    @click="toggleWire(`${turn.seq}:${wireIndex}`)"
                  >
                    <svg
                      width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6"
                      class="shrink-0 text-[#94a3b8] transition-transform"
                      :class="expandedWire.has(`${turn.seq}:${wireIndex}`) ? 'rotate-90' : ''"
                    >
                      <path d="M9 6l6 6-6 6" stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                    <span class="shrink-0 text-[11px] font-medium text-[#111827] dark:text-[#ececec]">#{{ wireIndex + 1 }}</span>
                    <span v-if="call.inputTokens != null || call.outputTokens != null" class="shrink-0 font-mono text-[10px] text-[#047857] dark:text-[#86efac]">
                      in {{ (call.inputTokens ?? 0).toLocaleString() }} / out {{ (call.outputTokens ?? 0).toLocaleString() }}
                    </span>
                    <span class="min-w-0 flex-1 truncate font-mono text-[10px] text-[#8a94a3] dark:text-[#858585]" :title="call.url">{{ call.url }}</span>
                  </button>
                  <div v-if="expandedWire.has(`${turn.seq}:${wireIndex}`)" class="flex flex-col gap-2 border-t border-[#e5e7eb] p-2 dark:border-[#3a3a3a]">
                    <!-- 发送 -->
                    <div>
                      <div class="mb-1 flex items-center gap-1.5">
                        <span class="text-[10px] font-semibold text-[#0369a1] dark:text-sky-300">发送</span>
                        <button type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(wireRequestText(call.request))">复制</button>
                      </div>
                      <div class="max-h-56 overflow-auto rounded bg-[#f3f4f6] p-1.5 dark:bg-[#2a2a2a]">
                        <JsonTree :data="call.request" :default-expand-depth="1" />
                      </div>
                    </div>
                    <!-- 接收 -->
                    <div>
                      <div class="mb-1 flex items-center gap-1.5">
                        <span class="text-[10px] font-semibold text-[#047857] dark:text-emerald-300">接收</span>
                        <button v-if="call.responseText" type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(call.responseText!)">复制</button>
                      </div>
                      <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-[#f3f4f6] p-1.5 font-mono text-[10px] leading-relaxed text-[#334155] dark:bg-[#2a2a2a] dark:text-[#d4d4d4]">{{ call.responseText ?? '（无：取消/出错）' }}</pre>
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <!-- 工具调用 -->
            <section v-if="turn.toolCalls.length > 0">
              <div class="mb-1 text-[11px] font-semibold text-[#334155] dark:text-[#ccc]">工具调用</div>
              <div class="flex flex-col gap-1">
                <div
                  v-for="tool in turn.toolCalls"
                  :key="tool.callId"
                  class="rounded-md border border-[#e5e7eb] bg-white dark:border-[#3a3a3a] dark:bg-[#1b1b1b]"
                >
                  <button
                    type="button"
                    class="flex w-full items-center gap-1.5 px-2 py-1.5 text-left"
                    @click="toggleTool(`${turn.seq}:${tool.callId}`)"
                  >
                    <span
                      class="h-1.5 w-1.5 shrink-0 rounded-full"
                      :class="tool.output == null
                        ? 'bg-sky-400'
                        : tool.isError ? 'bg-red-400' : 'bg-emerald-400'"
                    />
                    <span class="shrink-0 font-mono text-[11px] font-medium text-[#111827] dark:text-[#ececec]">{{ tool.toolName }}</span>
                    <span v-if="toolDuration(tool.startedAt, tool.finishedAt)" class="shrink-0 text-[10px] text-[#8a94a3] dark:text-[#858585]">
                      {{ toolDuration(tool.startedAt, tool.finishedAt) }}
                    </span>
                    <span class="min-w-0 flex-1 truncate font-mono text-[10px] text-[#8a94a3] dark:text-[#858585]">{{ tool.input }}</span>
                    <svg
                      width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"
                      class="shrink-0 text-[#94a3b8] transition-transform"
                      :class="expandedTools.has(`${turn.seq}:${tool.callId}`) ? 'rotate-180' : ''"
                    >
                      <path d="M6 9l6 6 6-6" stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                  </button>
                  <div v-if="expandedTools.has(`${turn.seq}:${tool.callId}`)" class="flex flex-col gap-1.5 border-t border-[#e5e7eb] p-2 dark:border-[#3a3a3a]">
                    <div>
                      <div class="mb-0.5 flex items-center gap-1.5">
                        <span class="text-[10px] font-medium text-[#64748b] dark:text-[#9ca3af]">入参</span>
                        <button type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(tool.input)">复制</button>
                      </div>
                      <pre class="max-h-32 overflow-auto whitespace-pre-wrap break-all rounded bg-[#f3f4f6] p-1.5 font-mono text-[10px] leading-relaxed text-[#334155] dark:bg-[#2a2a2a] dark:text-[#d4d4d4]">{{ previewText(tool.input) }}</pre>
                    </div>
                    <div>
                      <div class="mb-0.5 flex items-center gap-1.5">
                        <span class="text-[10px] font-medium text-[#64748b] dark:text-[#9ca3af]">
                          输出{{ tool.isError ? '（错误）' : '' }}
                        </span>
                        <button v-if="tool.output != null" type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(tool.output!)">复制</button>
                      </div>
                      <pre
                        class="max-h-32 overflow-auto whitespace-pre-wrap break-all rounded p-1.5 font-mono text-[10px] leading-relaxed"
                        :class="tool.isError
                          ? 'bg-red-50 text-red-700 dark:bg-red-500/10 dark:text-red-300'
                          : 'bg-[#f3f4f6] text-[#334155] dark:bg-[#2a2a2a] dark:text-[#d4d4d4]'"
                      >{{ tool.output == null ? '（执行中 / 无结果事件）' : previewText(tool.output) }}</pre>
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <!-- 助手输出 -->
            <section v-if="turn.assistantText">
              <div class="mb-1 flex items-center gap-1.5">
                <span class="text-[11px] font-semibold text-[#334155] dark:text-[#ccc]">助手输出</span>
                <button type="button" class="text-[10px] text-[#8a94a3] hover:text-[#334155] dark:hover:text-[#ccc]" @click="copyText(turn.assistantText)">复制</button>
              </div>
              <pre class="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md border border-[#e5e7eb] bg-white p-2 font-mono text-[11px] leading-relaxed text-[#111827] dark:border-[#3a3a3a] dark:bg-[#1b1b1b] dark:text-[#e5e5e5]">{{ previewText(turn.assistantText) }}</pre>
            </section>

            <!-- 推理过程 -->
            <section v-if="turn.reasoning">
              <details class="rounded-md border border-[#e5e7eb] bg-white dark:border-[#3a3a3a] dark:bg-[#1b1b1b]">
                <summary class="cursor-pointer px-2 py-1 text-[11px] text-[#64748b] dark:text-[#9ca3af]">
                  推理过程（{{ turn.reasoning.length.toLocaleString() }} 字符）
                </summary>
                <pre class="max-h-48 overflow-auto whitespace-pre-wrap break-words border-t border-[#e5e7eb] p-2 font-mono text-[10px] leading-relaxed text-[#64748b] dark:border-[#3a3a3a] dark:text-[#9ca3af]">{{ previewText(turn.reasoning, 2000) }}</pre>
              </details>
            </section>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
