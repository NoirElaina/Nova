<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, shallowRef } from 'vue';
import { useVirtualizer } from '@tanstack/vue-virtual';
import type {
  AgentMode,
  AskUserAnswerSubmission,
  AssistantTranscriptSegment,
  ChatMessage,
  ContextCompactSummary,
  ContextUsage,
  ConversationUsageSummary,
  NeedsUserInputPayload,
  PendingUploadFile,
  ToolExecutionEntry,
} from '../../lib/chat-types';
import type { LiveTurnStage } from '../../features/chat/controllers/chat-controller-types';
import InputArea from '../layout/InputArea.vue';
import AskUserInputDialog from './AskUserInputDialog.vue';
import AssistantMessageBubble from './messages/AssistantMessageBubble.vue';
import AssistantTranscript from './messages/AssistantTranscript.vue';
import ContextCompactNotice from './messages/ContextCompactNotice.vue';
import MessageTimelineNavigator from './MessageTimelineNavigator.vue';
import SubagentPanel from './SubagentPanel.vue';
import UserMessageBubble from './messages/UserMessageBubble.vue';
import { buildAssistantTranscriptSegments } from '../../features/chat/utils/assistant-transcript';
import { estimateTextTokens } from '../../features/chat/services/chat-api';

const props = defineProps<{
  messages: ChatMessage[];
  isGenerating: boolean;
  currentStage?: LiveTurnStage;
  assistantResponse: string;
  assistantReasoning?: string;
  assistantSegments: AssistantTranscriptSegment[];
  assistantTokenUsage?: number;
  /** 本轮开始时间（ms epoch）：发送时打点，页面刷新恢复时取后端 live_turns.startedAt */
  turnStartedAt?: number | null;
  currentTurnToolEntries: ToolExecutionEntry[];
  pendingQuestion?: NeedsUserInputPayload | null;
  pendingPermissionRequestId?: string | null;
  planMode?: boolean;
  agentMode?: AgentMode;
  pendingUploads?: PendingUploadFile[];
  contextUsage?: ContextUsage;
  contextCompacts?: ContextCompactSummary[];
  contextTokens?: number;
  conversationUsage?: ConversationUsageSummary | null;
  compacting?: boolean;
  /** AI 主流程错误原文：临时展示，不进入消息数组，下次发送时清空。 */
  chatError?: string | null;
  /** 当前对话挂载的智能体（会话级）。null = 默认 Nova（不展示）。 */
  activeAgent?: { id: string; name: string; description?: string } | null;
  /** 当前会话 id（插件命令展开 {workspace} 占位符用）。 */
  conversationId?: string | null;
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'remove-agent'): void;
  (e: 'save-user-edit', payload: { index: number; content: string; id?: string }): void;
  (e: 'ask-submit', value: AskUserAnswerSubmission): void;
  (e: 'ask-skip'): void;
  (e: 'cancel'): void;
  (e: 'mode-change', mode: AgentMode): void;
  (e: 'upload-files', files: PendingUploadFile[]): void;
  (e: 'remove-upload', index: number): void;
  (e: 'compact'): void;
  (e: 'dismiss-error'): void;
}>();

const chatAreaRef = ref<HTMLElement | null>(null);
const liveAssistantRef = ref<HTMLElement | null>(null);
const reactionMap = ref<Record<number, 'up' | 'down' | undefined>>({});
const copiedMap = ref<Record<string, boolean>>({});
const showScrollToBottom = ref(false);
/** 用户是否贴近底部；流式增高时只在 true 时跟滚，避免抢滚动 */
const stickToBottom = ref(true);
const activeUserMessageIndex = ref<number | null>(null);
const copyTimers: Record<string, ReturnType<typeof setTimeout> | undefined> = {};
let stickToBottomRaf = 0;

/** 会话 token 前缀和：conversationTokenUsage(i) = prefix[i] */
const tokenPrefixSums = shallowRef<number[]>([]);

const rebuildTokenPrefixSums = () => {
  const sums: number[] = new Array(props.messages.length);
  let running = 0;
  for (let i = 0; i < props.messages.length; i += 1) {
    const m = props.messages[i];
    const costTotal = (m.cost?.inputTokens ?? 0) + (m.cost?.outputTokens ?? 0);
    running += costTotal > 0 ? costTotal : (m.tokenUsage ?? 0);
    sums[i] = running;
  }
  tokenPrefixSums.value = sums;
};

watch(
  () => props.messages,
  () => rebuildTokenPrefixSums(),
  { immediate: true },
);

const formatNowTime = () => {
  const now = new Date();
  const hh = String(now.getHours()).padStart(2, '0');
  const mm = String(now.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
};

const formatMessageTime = (createdAt?: number) => {
  if (!createdAt || createdAt <= 0) {
    return formatNowTime();
  }
  const date = new Date(createdAt);
  if (Number.isNaN(date.getTime())) {
    return formatNowTime();
  }
  const hh = String(date.getHours()).padStart(2, '0');
  const mm = String(date.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
};

const copyText = async (text: string, key: string) => {
  if (!text?.trim()) return;
  try {
    await navigator.clipboard.writeText(text);
    copiedMap.value[key] = true;
    if (copyTimers[key]) {
      clearTimeout(copyTimers[key]);
    }
    copyTimers[key] = setTimeout(() => {
      copiedMap.value[key] = false;
    }, 900);
  } catch {
    // Ignore clipboard failures silently to keep UI interaction smooth.
  }
};

const setReaction = (index: number, value: 'up' | 'down') => {
  reactionMap.value[index] = reactionMap.value[index] === value ? undefined : value;
};

const retryFromUser = (index: number) => {
  const text = props.messages[index]?.content?.trim();
  if (!text) return;
  emit('send', text);
};

const retryFromAssistant = (assistantIndex: number) => {
  const prev = [...props.messages.slice(0, assistantIndex)].reverse().find((m) => m.role === 'user');
  if (!prev?.content?.trim()) return;
  emit('send', prev.content);
};

const buildAssistantCopyText = (message: ChatMessage) => {
  return message.content?.trim() || '';
};

const hasStreamingReasoning = () => !!props.assistantReasoning?.trim();
const streamingBodyText = () => props.assistantResponse.trim();

// 流式 segments 由 controller 维护；渲染前同样走 buildAssistantTranscriptSegments 的
// "按正文分组"合并：没有被正文分隔的 thinking/工具块合并展示，
// 保证流式中和回复完成后的分组视图一致，不会每轮思考/工具都单独成块。
const streamingSegments = computed(() => {
  if (props.assistantSegments.length > 0) {
    return buildAssistantTranscriptSegments(props.assistantSegments);
  }
  return buildAssistantTranscriptSegments([], {
    reasoning: props.assistantReasoning,
    text: props.assistantResponse,
  });
});

const hasLiveAssistantTurn = computed(
  () =>
    props.isGenerating ||
    streamingSegments.value.length > 0 ||
    props.currentTurnToolEntries.length > 0,
);

/** 虚拟列表行：历史消息 + 可选 live 行 */
type VirtualRow =
  | { kind: 'message'; index: number; message: ChatMessage }
  | { kind: 'live' };

const virtualRows = computed<VirtualRow[]>(() => {
  const rows: VirtualRow[] = props.messages.map((message, index) => ({
    kind: 'message',
    index,
    message,
  }));
  if (hasLiveAssistantTurn.value) {
    rows.push({ kind: 'live' });
  }
  return rows;
});

const rowVirtualizer = useVirtualizer(
  computed(() => ({
    count: virtualRows.value.length,
    getScrollElement: () => chatAreaRef.value,
    estimateSize: () => 160,
    overscan: 10,
    getItemKey: (index: number) => {
      const row = virtualRows.value[index];
      if (!row) return index;
      if (row.kind === 'live') return 'live-assistant';
      return row.message.id || `msg-${row.index}`;
    },
  })),
);

const virtualItems = computed(() => rowVirtualizer.value.getVirtualItems());
const totalSize = computed(() => rowVirtualizer.value.getTotalSize());

const measureElement = (el: unknown) => {
  if (el instanceof Element) {
    rowVirtualizer.value.measureElement(el);
  }
};

const scrollToBottom = async () => {
  await nextTick();
  stickToBottom.value = true;
  const count = virtualRows.value.length;
  if (count > 0 && chatAreaRef.value) {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
    // 再对齐一次，等 virtualizer 用真实高度算完 totalSize
    requestAnimationFrame(() => {
      if (chatAreaRef.value) {
        chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
      }
    });
  } else if (chatAreaRef.value) {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
  }
  updateScrollToBottomVisibility();
};

const scrollLastUserMessageToTop = async () => {
  await nextTick();
  let lastUser = -1;
  for (let i = props.messages.length - 1; i >= 0; i -= 1) {
    if (props.messages[i]?.role === 'user') {
      lastUser = i;
      break;
    }
  }
  if (lastUser >= 0) {
    rowVirtualizer.value.scrollToIndex(lastUser, { align: 'start' });
  } else {
    await scrollToBottom();
  }
};

const scrollLastUserMessageToBottom = async () => {
  await nextTick();
  let lastUser = -1;
  for (let i = props.messages.length - 1; i >= 0; i -= 1) {
    if (props.messages[i]?.role === 'user') {
      lastUser = i;
      break;
    }
  }
  if (lastUser >= 0) {
    rowVirtualizer.value.scrollToIndex(lastUser, { align: 'end' });
  } else {
    await scrollToBottom();
  }
  updateScrollToBottomVisibility();
};

const scrollLiveAssistantIntoView = async () => {
  await nextTick();
  // 新一轮生成：贴底跟滚，不要 align:start（表格/长文增高时会把视口顶来顶去）
  stickToBottom.value = true;
  pinToBottomIfSticky();
  updateScrollToBottomVisibility();
};

const distanceFromBottomPx = () => {
  const el = chatAreaRef.value;
  if (!el) return 0;
  return el.scrollHeight - el.clientHeight - el.scrollTop;
};

const updateScrollToBottomVisibility = () => {
  if (!chatAreaRef.value) {
    showScrollToBottom.value = false;
    return;
  }
  const distance = distanceFromBottomPx();
  // 只有真正贴近底部才算"贴底"；阈值太大会在用户刚往上滚一点时
  // 被流式跟滚反复拉回底部，产生"拉扯好几下才能上去"的体感。
  stickToBottom.value = distance <= 40;
  showScrollToBottom.value = distance > 120;
};

/** 滚轮手势感知：用户主动上滑时立刻解除贴底跟滚，避免流式内容把视口拽回去 */
const handleChatWheel = (event: WheelEvent) => {
  if (event.deltaY < 0) {
    // 只解除跟滚；此刻滚动尚未生效，不能立刻重算距离，否则又会被判回贴底
    stickToBottom.value = false;
  } else if (event.deltaY > 0) {
    // 下滑滚回底部附近时恢复跟滚
    updateScrollToBottomVisibility();
  }
};

/** 仅贴底时把视口钉在列表末尾；不调用 measure()，避免清空虚拟列表尺寸缓存导致狂抖 */
const pinToBottomIfSticky = () => {
  if (!stickToBottom.value || !chatAreaRef.value) return;
  const count = virtualRows.value.length;
  if (count <= 0) return;
  // 直接改 scrollTop 比 scrollToIndex 更稳：不触发额外 layout 估算抖动
  const el = chatAreaRef.value;
  el.scrollTop = el.scrollHeight;
  showScrollToBottom.value = false;
};

const summarizeUserMessage = (content: string) => {
  const normalized = content.replace(/\s+/g, ' ').trim();
  if (!normalized) return '空消息';
  return normalized.length > 56 ? `${normalized.slice(0, 56)}...` : normalized;
};

const userTimelineItems = computed(() =>
  props.messages
    .map((message, index) => ({ message, index }))
    .filter(({ message }) => message.role === 'user' && message.content.trim())
    .map(({ message, index }) => ({
      index,
      summary: summarizeUserMessage(message.content),
    })),
);

const updateActiveUserMessage = () => {
  const container = chatAreaRef.value;
  if (!container || userTimelineItems.value.length === 0) {
    activeUserMessageIndex.value = null;
    return;
  }

  const rows = Array.from(
    container.querySelectorAll<HTMLElement>('[data-role="user"][data-message-index]'),
  );
  if (rows.length === 0) {
    // 虚拟列表可能未挂载目标，按滚动比例估算
    const items = userTimelineItems.value;
    if (items.length === 0) {
      activeUserMessageIndex.value = null;
      return;
    }
    const maxScroll = Math.max(1, container.scrollHeight - container.clientHeight);
    const ratio = container.scrollTop / maxScroll;
    const approx = Math.min(items.length - 1, Math.floor(ratio * items.length));
    activeUserMessageIndex.value = items[approx]?.index ?? null;
    return;
  }

  const containerTop = container.getBoundingClientRect().top;
  let closestIndex: number | null = null;
  let closestDistance = Number.POSITIVE_INFINITY;

  for (const row of rows) {
    const rawIndex = row.dataset.messageIndex;
    if (!rawIndex) continue;
    const index = Number.parseInt(rawIndex, 10);
    if (!Number.isFinite(index)) continue;

    const distance = Math.abs(row.getBoundingClientRect().top - containerTop - 20);
    if (distance < closestDistance) {
      closestDistance = distance;
      closestIndex = index;
    }
  }

  activeUserMessageIndex.value = closestIndex;
};

const handleChatScroll = () => {
  updateScrollToBottomVisibility();
  updateActiveUserMessage();
};

const scrollToBottomSmooth = async () => {
  await nextTick();
  stickToBottom.value = true;
  if (chatAreaRef.value) {
    chatAreaRef.value.scrollTo({
      top: chatAreaRef.value.scrollHeight,
      behavior: 'smooth',
    });
  }
};

const scrollToMessageIndex = async (index: number) => {
  await nextTick();
  if (index < 0 || index >= props.messages.length) return;
  activeUserMessageIndex.value = index;
  rowVirtualizer.value.scrollToIndex(index, { align: 'start', behavior: 'smooth' });
};

onMounted(() => {
  stickToBottom.value = true;
  void scrollToBottom();
  void nextTick(updateActiveUserMessage);
});

onBeforeUnmount(() => {
  if (stickToBottomRaf) {
    cancelAnimationFrame(stickToBottomRaf);
    stickToBottomRaf = 0;
  }
  if (streamingEstimateTimer !== null) {
    clearTimeout(streamingEstimateTimer);
    streamingEstimateTimer = null;
  }
  stopLiveElapsedTimer();
  for (const key of Object.keys(copyTimers)) {
    if (copyTimers[key]) {
      clearTimeout(copyTimers[key]);
    }
  }
});

// 结构变化（消息条数/是否出现 live 行）才允许轻量同步 UI；
// 禁止在每个 token 上 virtualizer.measure()——那会清空尺寸缓存，
// 历史行在 estimate(160) 与真实高度间反复横跳，滚动条上下抽搐。
watch(
  () => [props.messages.length, hasLiveAssistantTurn.value] as const,
  async () => {
    await nextTick();
    updateScrollToBottomVisibility();
    updateActiveUserMessage();
    if (stickToBottom.value) {
      pinToBottomIfSticky();
    }
  },
);

// 流式正文/工具输出增高：rAF 合并，仅贴底时跟滚；高度交给 measureElement 的 ResizeObserver。
watch(
  () =>
    [
      props.assistantResponse.length,
      props.assistantReasoning?.length ?? 0,
      props.assistantSegments.length,
      props.currentTurnToolEntries.length,
      props.isGenerating,
    ] as const,
  async () => {
    if (stickToBottomRaf) return;
    stickToBottomRaf = requestAnimationFrame(async () => {
      stickToBottomRaf = 0;
      await nextTick();
      if (stickToBottom.value) {
        pinToBottomIfSticky();
      } else {
        updateScrollToBottomVisibility();
      }
    });
  },
);

const handleSend = (msg: string) => {
  emit('send', msg);
};

const handleUploadFiles = (files: PendingUploadFile[]) => {
  emit('upload-files', files);
};

const handleRemoveUpload = (index: number) => {
  emit('remove-upload', index);
};

const conversationTokenUsage = (index: number): number => {
  return tokenPrefixSums.value[index] ?? 0;
};

/** 流式输出期间的临时 token 估算：节流调后端标准计数器，真实 usage 到达后由 assistantTokenUsage 覆盖 */
const streamingEstimateTokens = ref(0);
let streamingEstimateTimer: ReturnType<typeof setTimeout> | null = null;

const refreshStreamingEstimate = () => {
  if (streamingEstimateTimer !== null) return;
  streamingEstimateTimer = setTimeout(() => {
    streamingEstimateTimer = null;
    const text = props.assistantResponse;
    if (!text.trim() || !props.isGenerating) return;
    if (typeof props.assistantTokenUsage === 'number' && props.assistantTokenUsage > 0) return;
    void estimateTextTokens(text)
      .then((tokens) => {
        streamingEstimateTokens.value = tokens;
      })
      .catch(() => 0);
  }, 800);
};

watch(
  () => [props.assistantResponse, props.isGenerating] as const,
  ([, generating]) => {
    if (generating) {
      refreshStreamingEstimate();
      return;
    }
    if (streamingEstimateTimer !== null) {
      clearTimeout(streamingEstimateTimer);
      streamingEstimateTimer = null;
    }
    streamingEstimateTokens.value = 0;
  },
);

/** 本轮实时执行时长：本地定时器刷新，起点来自 controller 的 turnStartedAt */
const liveElapsedMs = ref(0);
let liveElapsedTimer: ReturnType<typeof setInterval> | null = null;

const formatElapsedMs = (ms: number): string => {
  const totalSeconds = Math.floor(ms / 1000);
  if (totalSeconds < 60) {
    return `${(ms / 1000).toFixed(1)}s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) {
    return `${minutes}m${String(seconds).padStart(2, '0')}s`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours}h${String(minutes % 60).padStart(2, '0')}m`;
};

const stopLiveElapsedTimer = () => {
  if (liveElapsedTimer !== null) {
    clearInterval(liveElapsedTimer);
    liveElapsedTimer = null;
  }
  liveElapsedMs.value = 0;
};

watch(
  () => [props.isGenerating, props.turnStartedAt] as const,
  ([generating, startedAt]) => {
    if (generating && startedAt && startedAt > 0) {
      liveElapsedMs.value = Math.max(0, Date.now() - startedAt);
      if (liveElapsedTimer === null) {
        liveElapsedTimer = setInterval(() => {
          const current = props.turnStartedAt;
          if (props.isGenerating && current && current > 0) {
            liveElapsedMs.value = Math.max(0, Date.now() - current);
            return;
          }
          stopLiveElapsedTimer();
        }, 250);
      }
      return;
    }
    stopLiveElapsedTimer();
  },
  { immediate: true },
);

const streamingTokenUsage = (): number => {
  const outputTokens =
    typeof props.assistantTokenUsage === 'number' && props.assistantTokenUsage > 0
      ? props.assistantTokenUsage
      : props.isGenerating
        ? streamingEstimateTokens.value
        : 0;
  const inputTokens =
    typeof props.contextUsage?.usedTokens === 'number' && props.contextUsage.usedTokens > 0
      ? props.contextUsage.usedTokens
      : props.contextTokens ?? 0;
  const total = inputTokens + outputTokens;
  if (total > 0) {
    return total;
  }
  return 0;
};

const streamingConversationTokenUsage = (): number => {
  const base =
    tokenPrefixSums.value.length > 0
      ? tokenPrefixSums.value[tokenPrefixSums.value.length - 1]
      : 0;
  return base + streamingTokenUsage();
};

const liveWaitKind = () => {
  if (!props.pendingQuestion) return null;
  return props.pendingPermissionRequestId ? 'permission' : 'question';
};

const liveStatusText = computed(() => {
  if (props.currentStage === 'compacting') {
    return '正在压缩上下文';
  }
  const waitKind = liveWaitKind();
  if (waitKind === 'permission') {
    return '等待你确认工具权限';
  }
  if (waitKind === 'question') {
    return '等待你补充信息';
  }
  const runningTool = props.currentTurnToolEntries.find((entry) => entry.status === 'running');
  if (runningTool) {
    const name = runningTool.toolName.toLowerCase();
    if (
      name.includes('read') ||
      name.includes('file') ||
      name.includes('rag') ||
      name.includes('document')
    ) {
      return '正在读文件';
    }
    if (
      name.includes('bash') ||
      name.includes('powershell') ||
      name.includes('shell') ||
      name.includes('command')
    ) {
      return '正在执行命令';
    }
    if (name.includes('compact')) {
      return '正在压缩上下文';
    }
    return `正在调用工具：${runningTool.toolName}`;
  }
  const hasFinishedTool = props.currentTurnToolEntries.some((entry) => entry.status !== 'running');
  if (hasFinishedTool && !streamingBodyText()) {
    return '等待模型总结';
  }
  if (hasStreamingReasoning() && !streamingBodyText()) {
    return '正在思考';
  }
  if (props.isGenerating) {
    return '正在生成回复';
  }
  return '正在处理你的请求';
});

function messageRowAt(virtualIndex: number) {
  const row = virtualRows.value[virtualIndex];
  if (!row || row.kind !== 'message') return null;
  return row;
}

defineExpose({
  scrollToBottom,
  scrollLastUserMessageToTop,
  scrollLastUserMessageToBottom,
  scrollLiveAssistantIntoView,
});
</script>

<template>
  <div class="relative flex flex-col h-full w-full max-w-4xl mx-auto pt-14">
    <div
      class="chat-scroll-area flex-1 overflow-y-auto px-4 pb-4 custom-scrollbar"
      ref="chatAreaRef"
      @scroll.passive="handleChatScroll"
      @wheel.passive="handleChatWheel"
    >
      <div
        class="w-full relative"
        :style="{ height: `${totalSize}px` }"
      >
        <div
          v-for="vItem in virtualItems"
          :key="String(vItem.key)"
          :ref="measureElement"
          :data-index="vItem.index"
          class="absolute left-0 w-full pb-6"
          :style="{
            transform: `translateY(${vItem.start}px)`,
          }"
        >
          <template v-if="messageRowAt(vItem.index)">
            <div
              class="flex w-full group"
              :data-role="messageRowAt(vItem.index)!.message.role"
              :data-message-index="messageRowAt(vItem.index)!.index"
            >
              <UserMessageBubble
                v-if="messageRowAt(vItem.index)!.message.role === 'user'"
                :message="messageRowAt(vItem.index)!.message"
                :index="messageRowAt(vItem.index)!.index"
                :copied="!!copiedMap[`user-${messageRowAt(vItem.index)!.index}`]"
                :timeText="formatMessageTime(messageRowAt(vItem.index)!.message.createdAt)"
                @retry="retryFromUser"
                @save-edit="emit('save-user-edit', $event)"
                @copy="copyText(messageRowAt(vItem.index)!.message.content, `user-${messageRowAt(vItem.index)!.index}`)"
              />

              <AssistantMessageBubble
                v-else
                :message="messageRowAt(vItem.index)!.message"
                :index="messageRowAt(vItem.index)!.index"
                :copied="!!copiedMap[`assistant-${messageRowAt(vItem.index)!.index}`]"
                :conversationTokenUsage="conversationTokenUsage(messageRowAt(vItem.index)!.index)"
                :reaction="reactionMap[messageRowAt(vItem.index)!.index]"
                @copy="copyText(buildAssistantCopyText(messageRowAt(vItem.index)!.message), `assistant-${messageRowAt(vItem.index)!.index}`)"
                @retry="retryFromAssistant"
                @react="setReaction($event.index, $event.value)"
              />
            </div>
          </template>

          <div
            v-else-if="virtualRows[vItem.index]?.kind === 'live'"
            ref="liveAssistantRef"
            class="flex w-full justify-start group"
            data-role="assistant-live"
          >
            <div class="w-full max-w-[85%]">
              <div class="min-w-0 flex-1 text-[0.95rem] leading-relaxed break-words text-[#1a1a1a] dark:text-[#ececec]">
                <ContextCompactNotice
                  v-if="props.contextCompacts && props.contextCompacts.length > 0"
                  :items="props.contextCompacts"
                  compact
                />
                <AssistantTranscript
                  v-if="streamingSegments.length > 0"
                  :segments="streamingSegments"
                  :entries="props.currentTurnToolEntries"
                  live
                />
                <p
                  v-else-if="props.isGenerating || !!liveWaitKind()"
                  class="live-status text-[13px] text-[#64748b] dark:text-[#cbd5e1]"
                >
                  <span>{{ liveStatusText }}</span>
                  <span class="live-status-dots" aria-hidden="true">
                    <span></span>
                    <span></span>
                    <span></span>
                  </span>
                </p>
                <span
                  v-if="isGenerating"
                  class="inline-block w-1.5 h-[1em] bg-current ml-1 align-middle animate-pulse opacity-70"
                ></span>
                <div
                  v-if="liveElapsedMs > 0 || streamingTokenUsage() > 0 || streamingConversationTokenUsage() > 0"
                  class="mt-2 flex flex-wrap items-center gap-2"
                >
                  <span v-if="liveElapsedMs > 0" class="token-badge">
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <circle cx="12" cy="12" r="10"></circle>
                      <polyline points="12 6 12 12 15 14"></polyline>
                    </svg>
                    {{ formatElapsedMs(liveElapsedMs) }}
                  </span>
                  <span
                    v-if="streamingTokenUsage() > 0 || streamingConversationTokenUsage() > 0"
                    class="token-badge"
                  >
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                      <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
                      <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
                      <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
                    </svg>
                    本次 {{ streamingTokenUsage() }} · 会话 {{ streamingConversationTokenUsage() }}
                  </span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <MessageTimelineNavigator
      :items="userTimelineItems"
      :activeIndex="activeUserMessageIndex"
      @select="scrollToMessageIndex"
    />

    <SubagentPanel :conversation-id="conversationId" />

    <button
      v-if="showScrollToBottom"
      type="button"
      class="scroll-to-bottom-btn"
      aria-label="滚动到底部"
      title="回到底部"
      @click="scrollToBottomSmooth"
    >
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 5v14" />
        <path d="m5 12 7 7 7-7" />
      </svg>
    </button>

    <div
      v-if="chatError"
      class="w-full px-4 pt-3"
    >
      <div class="mx-auto w-full max-w-[900px] rounded-lg border border-red-300 bg-red-50 px-4 py-3 text-red-700 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0 flex-1">
            <div class="mb-1 flex items-center gap-1.5 text-xs font-semibold">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="12" cy="12" r="10" />
                <path d="M12 8v4" />
                <path d="M12 16h.01" />
              </svg>
              报错
            </div>
            <pre class="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">{{ chatError }}</pre>
          </div>
          <button
            type="button"
            class="shrink-0 rounded p-0.5 opacity-70 transition-opacity hover:opacity-100"
            title="关闭"
            aria-label="关闭错误提示"
            @click="emit('dismiss-error')"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      </div>
    </div>

    <div class="w-full bg-transparent px-4 pt-6 pb-6">
      <div class="w-full max-w-[900px] mx-auto">
        <AskUserInputDialog
          v-if="pendingQuestion"
          :request="pendingQuestion"
          @submit="emit('ask-submit', $event)"
          @skip="emit('ask-skip')"
        />
        <InputArea
          v-else
          :isGenerating="isGenerating"
          :agentMode="agentMode"
          :pendingUploads="pendingUploads"
          :contextUsage="contextUsage"
          :contextTokens="contextTokens"
          :conversationUsage="conversationUsage"
          :compacting="compacting"
          :activeAgent="activeAgent"
          :conversationId="conversationId"
          @send="handleSend"
          @cancel="emit('cancel')"
          @remove-agent="emit('remove-agent')"
          @mode-change="emit('mode-change', $event)"
          @upload-files="handleUploadFiles"
          @remove-upload="handleRemoveUpload"
          @compact="emit('compact')"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-scroll-area {
  position: relative;
  overflow-anchor: none;
  scrollbar-gutter: stable;
}

.custom-scrollbar::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}

.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: var(--color-border, #e5e5e5);
  border-radius: 10px;
}

.dark .custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: #444;
}

.scroll-to-bottom-btn {
  position: absolute;
  left: 50%;
  bottom: 174px;
  width: 34px;
  height: 34px;
  border: 1px solid rgba(203, 213, 225, 0.92);
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.96);
  color: #111827;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 14px 30px rgba(15, 23, 42, 0.12), 0 2px 6px rgba(15, 23, 42, 0.06);
  backdrop-filter: blur(10px);
  cursor: pointer;
  z-index: 8;
  transform: translateX(-50%);
  transition: transform 0.18s ease, box-shadow 0.18s ease, border-color 0.18s ease;
}

.scroll-to-bottom-btn:hover {
  transform: translateX(-50%) translateY(-2px);
  box-shadow: 0 18px 34px rgba(15, 23, 42, 0.16), 0 4px 10px rgba(15, 23, 42, 0.1);
  border-color: rgba(148, 163, 184, 0.75);
}

.scroll-to-bottom-btn:focus-visible {
  outline: 2px solid rgba(37, 99, 235, 0.24);
  outline-offset: 3px;
}

.dark .scroll-to-bottom-btn {
  background: rgba(31, 41, 55, 0.96);
  color: #f8fafc;
  border-color: rgba(71, 85, 105, 0.95);
  box-shadow: 0 14px 30px rgba(0, 0, 0, 0.34), 0 2px 6px rgba(0, 0, 0, 0.18);
}

.dark .scroll-to-bottom-btn:hover {
  border-color: rgba(148, 163, 184, 0.68);
  box-shadow: 0 18px 34px rgba(0, 0, 0, 0.42), 0 4px 10px rgba(0, 0, 0, 0.24);
}

@media (max-width: 900px) {
  .scroll-to-bottom-btn {
    bottom: 156px;
    width: 32px;
    height: 32px;
  }
}

.token-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 9px;
  color: #047857;
  border: 1px solid #a7f3d0;
  background: #ecfdf5;
  padding: 3px 6px;
  border-radius: 6px;
  font-family: monospace;
  letter-spacing: 0.04em;
  font-variant-numeric: tabular-nums;
}

.dark .token-badge {
  color: #86efac;
  border-color: rgba(34, 197, 94, 0.38);
  background: rgba(20, 83, 45, 0.32);
}

.live-status {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.live-status-dots {
  display: inline-flex;
  align-items: flex-end;
  gap: 5px;
  min-width: 24px;
}

.live-status-dots span {
  width: 5px;
  height: 5px;
  border-radius: 999px;
  background: currentColor;
  opacity: 0.45;
  animation: live-status-bounce 1s ease-in-out infinite;
}

.live-status-dots span:nth-child(2) {
  animation-delay: 0.15s;
}

.live-status-dots span:nth-child(3) {
  animation-delay: 0.3s;
}

@keyframes live-status-bounce {
  0%, 80%, 100% {
    transform: translateY(0) scale(0.92);
    opacity: 0.35;
  }
  40% {
    transform: translateY(-4px) scale(1);
    opacity: 0.95;
  }
}

</style>
