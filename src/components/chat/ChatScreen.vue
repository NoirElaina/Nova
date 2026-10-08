<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, shallowRef } from 'vue';
import { useVirtualizer } from '@tanstack/vue-virtual';
import type {
  AskUserAnswerSubmission,
  ChatMessage,
} from '../../lib/chat-types';
import InputArea from './input/InputArea.vue';
import AskUserInputDialog from './AskUserInputDialog.vue';
import PlanChip from './PlanChip.vue';
import TodoChip from './TodoChip.vue';
import BackgroundJobsChip from './BackgroundJobsChip.vue';
import AssistantMessageBubble from './messages/AssistantMessageBubble.vue';
import ChatActiveTurnBar from './ChatActiveTurnBar.vue';
import BranchSidebar from './branch/BranchSidebar.vue';
import MessageTimelineNavigator from './MessageTimelineNavigator.vue';
import SelectionActionPopover from './SelectionActionPopover.vue';
import SubagentPanel from './SubagentPanel.vue';
import UserMessageBubble from './messages/UserMessageBubble.vue';
import { buildAssistantTranscriptSegments } from '../../features/chat/utils/assistant-transcript';
import { estimateTextTokens } from '../../features/chat/services/chat-api';
import { initBranchEvents } from '../../features/branch/branch-chat';

import { useConversationStore } from '@/stores/conversation';
import { useAgentSessionStore } from '@/stores/agentSession';
import { useSelectionPopover } from '@/composables/useSelectionPopover';
import { useChatScroll } from '@/composables/useChatScroll';


defineProps<{
  /** 当前对话挂载的智能体（会话级）。null = 默认 Nova（不展示）。 */
  activeAgent?: { id: string; name: string; description?: string } | null;
  /** 右侧工作区抽屉展开时隐藏消息时间线导航（避免遮挡收窄后的聊天内容）。 */
  drawerOpen?: boolean;
}>();

const conversationStore = useConversationStore();
const sessionStore = useAgentSessionStore();

const messages = computed(() => sessionStore.activeSession.messages);
const isGenerating = computed(() => sessionStore.activeSession.isGenerating);
const currentStage = computed(() => sessionStore.activeSession.currentStage);
const assistantResponse = computed(() => sessionStore.activeSession.assistantResponse);
const assistantReasoning = computed(() => sessionStore.activeSession.assistantReasoning);
const assistantSegments = computed(() => sessionStore.activeSession.assistantSegments);
const assistantTokenUsage = computed(() => sessionStore.activeSession.assistantTokenUsage);
const turnStartedAt = computed(() => sessionStore.activeSession.currentTurnStartedAt);
const currentTurnToolEntries = computed(() => sessionStore.activeSession.toolExecutionLogs);
const pendingQuestion = computed(() => sessionStore.activeSession.pendingQuestion);
const pendingPermissionRequestId = computed(() => sessionStore.activeSession.pendingPermissionRequestId);
const contextUsage = computed(() => sessionStore.activeSession.contextUsage);
const contextCompacts = computed(() => sessionStore.activeSession.contextCompacts);
const contextTokens = computed(() => sessionStore.activeSession.contextTokens);
const chatError = computed(() => sessionStore.activeSession.chatError);
const conversationId = computed(() => conversationStore.activeConversationId);

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'remove-agent'): void;
  (e: 'save-user-edit', payload: { index: number; content: string; id?: string }): void;
  (e: 'ask-submit', value: AskUserAnswerSubmission): void;
  (e: 'ask-skip'): void;
  (e: 'cancel'): void;
  (e: 'compact'): void;
  (e: 'dismiss-error'): void;
  (e: 'open-plan'): void;
  (e: 'open-background-jobs'): void;
}>();

const chatAreaRef = ref<HTMLElement | null>(null);
const liveAssistantRef = ref<HTMLElement | null>(null);
const inputAreaRef = ref<InstanceType<typeof InputArea> | null>(null);

const {
  selectionPopover,
  hideSelectionPopover,
  handleChatMouseUp,
  handleQuoteToInput,
  handleOpenBranch,
} = useSelectionPopover({
  chatAreaRef,
  conversationId,
  onQuote: (text) => inputAreaRef.value?.insertQuotedText(text),
});

const reactionMap = ref<Record<number, 'up' | 'down' | undefined>>({});
const copiedMap = ref<Record<string, boolean>>({});
const copyTimers: Record<string, ReturnType<typeof setTimeout> | undefined> = {};
let stickToBottomRaf = 0;


/** 会话 token 前缀和：conversationTokenUsage(i) = prefix[i] */
const tokenPrefixSums = shallowRef<number[]>([]);

const rebuildTokenPrefixSums = () => {
  const sums: number[] = new Array(messages.value.length);
  let running = 0;
  for (let i = 0; i < messages.value.length; i += 1) {
    const m = messages.value[i];
    const costTotal = (m.cost?.inputTokens ?? 0) + (m.cost?.outputTokens ?? 0);
    running += costTotal > 0 ? costTotal : (m.tokenUsage ?? 0);
    sums[i] = running;
  }
  tokenPrefixSums.value = sums;
};

watch(
  messages,
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
  const text = messages.value[index]?.content?.trim();
  if (!text) return;
  emit('send', text);
};

const retryFromAssistant = (assistantIndex: number) => {
  const prev = [...messages.value.slice(0, assistantIndex)].reverse().find((m) => m.role === 'user');
  if (!prev?.content?.trim()) return;
  emit('send', prev.content);
};

const buildAssistantCopyText = (message: ChatMessage) => {
  return message.content?.trim() || '';
};

const hasStreamingReasoning = () => !!assistantReasoning.value?.trim();
const streamingBodyText = () => assistantResponse.value.trim();

// 流式 segments 由 controller 维护；渲染前同样走 buildAssistantTranscriptSegments 的
// "按正文分组"合并：没有被正文分隔的 thinking/工具块合并展示，
// 保证流式中和回复完成后的分组视图一致，不会每轮思考/工具都单独成块。
const streamingSegments = computed(() => {
  if (assistantSegments.value.length > 0) {
    return buildAssistantTranscriptSegments(assistantSegments.value);
  }
  return buildAssistantTranscriptSegments([], {
    reasoning: assistantReasoning.value,
    text: assistantResponse.value,
  });
});

const hasLiveAssistantTurn = computed(() => {
  if (isGenerating.value) {
    return true;
  }
  const lastMsg = messages.value[messages.value.length - 1];
  if (lastMsg && lastMsg.role === 'assistant') {
    return false;
  }
  return streamingSegments.value.length > 0 || currentTurnToolEntries.value.length > 0;
});

/** 虚拟列表行：历史消息 + 可选 live 行 */
type VirtualRow =
  | { kind: 'message'; index: number; message: ChatMessage }
  | { kind: 'live' };

const virtualRows = computed<VirtualRow[]>(() => {
  const rows: VirtualRow[] = messages.value.map((message, index) => ({
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

const summarizeUserMessage = (content: string) => {
  const normalized = content.replace(/\s+/g, ' ').trim();
  if (!normalized) return '空消息';
  return normalized.length > 56 ? `${normalized.slice(0, 56)}...` : normalized;
};

const userTimelineItems = computed(() =>
  messages.value
    .map((message, index) => ({ message, index }))
    .filter(({ message }) => message.role === 'user' && message.content.trim())
    .map(({ message, index }) => ({
      index,
      summary: summarizeUserMessage(message.content),
    })),
);

const {
  showScrollToBottom,
  stickToBottom,
  activeUserMessageIndex,
  updateScrollToBottomVisibility,
  handleChatWheel,
  pinToBottomIfSticky,
  scrollToBottom,
  scrollLastUserMessageToTop,
  scrollLastUserMessageToBottom,
  scrollLiveAssistantIntoView,
  updateActiveUserMessage,
  handleChatScroll,
  scrollToBottomSmooth,
  scrollToMessageIndex,
} = useChatScroll({
  chatAreaRef,
  virtualRowsCount: computed(() => virtualRows.value.length),
  rowVirtualizer,
  messages,
  userTimelineItems,
  onScrolled: hideSelectionPopover,
});

onMounted(() => {
  stickToBottom.value = true;
  void scrollToBottom();
  void nextTick(updateActiveUserMessage);
  void initBranchEvents();
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
  () => [messages.value.length, hasLiveAssistantTurn.value] as const,
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
      assistantResponse.value.length,
      assistantReasoning.value?.length ?? 0,
      assistantSegments.value.length,
      currentTurnToolEntries.value.length,
      isGenerating.value,
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
    const text = assistantResponse.value;
    if (!text.trim() || !isGenerating.value) return;
    if (typeof assistantTokenUsage.value === 'number' && assistantTokenUsage.value > 0) return;
    void estimateTextTokens(text)
      .then((tokens) => {
        streamingEstimateTokens.value = tokens;
      })
      .catch(() => 0);
  }, 800);
};

watch(
  () => [assistantResponse.value, isGenerating.value] as const,
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
  () => [isGenerating.value, turnStartedAt.value] as const,
  ([generating, startedAt]) => {
    if (generating && startedAt && startedAt > 0) {
      liveElapsedMs.value = Math.max(0, Date.now() - startedAt);
      if (liveElapsedTimer === null) {
        liveElapsedTimer = setInterval(() => {
          const current = turnStartedAt.value;
          if (isGenerating.value && current && current > 0) {
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
    typeof assistantTokenUsage.value === 'number' && assistantTokenUsage.value > 0
      ? assistantTokenUsage.value
      : isGenerating.value
        ? streamingEstimateTokens.value
        : 0;
  const inputTokens =
    typeof contextUsage.value?.usedTokens === 'number' && contextUsage.value.usedTokens > 0
      ? contextUsage.value.usedTokens
      : contextTokens.value ?? 0;
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
  if (!pendingQuestion.value) return null;
  return pendingPermissionRequestId.value ? 'permission' : 'question';
};

const liveStatusText = computed(() => {
  const cogState = sessionStore.activeSession.cognitiveState;
  if (cogState === 'verifying_workspace') {
    return '正在验证代码语法有效性';
  }
  if (cogState === 'reflecting') {
    return '分析诊断信息并自愈修复中';
  }
  if (cogState === 'assembling_context') {
    return '正在装配认知上下文';
  }
  if (currentStage.value === 'compacting') {
    return '正在压缩上下文';
  }
  const waitKind = liveWaitKind();
  if (waitKind === 'permission') {
    return '等待你确认工具权限';
  }
  if (waitKind === 'question') {
    return '等待你补充信息';
  }
  const runningTool = currentTurnToolEntries.value.find((entry) => entry.status === 'running');
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
  const hasFinishedTool = currentTurnToolEntries.value.some((entry) => entry.status !== 'running');
  if (hasFinishedTool && !streamingBodyText()) {
    return '等待模型总结';
  }
  if (hasStreamingReasoning() && !streamingBodyText()) {
    return '正在思考';
  }
  if (isGenerating.value) {
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
  <div class="relative h-full w-full">
    <!-- 消息时间线导航：锚定聊天区容器左边缘（不随居中内容列浮动） -->
    <MessageTimelineNavigator
      v-if="!drawerOpen"
      :items="userTimelineItems"
      :activeIndex="activeUserMessageIndex"
      @select="scrollToMessageIndex"
    />
  <div class="relative flex flex-col h-full w-full max-w-4xl mx-auto pt-14">
    <div
      class="chat-scroll-area flex-1 overflow-y-auto px-4 pb-4 custom-scrollbar"
      ref="chatAreaRef"
      @scroll.passive="handleChatScroll"
      @wheel.passive="handleChatWheel"
      @mouseup="handleChatMouseUp"
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
                :entries="currentTurnToolEntries"
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
          >
            <ChatActiveTurnBar
              :contextCompacts="contextCompacts"
              :streamingSegments="streamingSegments"
              :currentTurnToolEntries="currentTurnToolEntries"
              :isGenerating="isGenerating"
              :liveWaitKind="liveWaitKind()"
              :liveStatusText="liveStatusText"
              :liveElapsedMs="liveElapsedMs"
              :streamingTokenUsage="streamingTokenUsage()"
              :streamingConversationTokenUsage="streamingConversationTokenUsage()"
              :formatElapsedMs="formatElapsedMs"
            />
          </div>
        </div>
      </div>
    </div>

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
        <!-- 输入框上方小框：计划 / 任务进度，与工具栏同规格的紧凑控件 -->
        <div class="mb-1.5 flex flex-wrap items-center gap-1.5">
          <PlanChip :conversationId="conversationId" @open="emit('open-plan')" />
          <TodoChip :conversationId="conversationId" />
          <BackgroundJobsChip :conversationId="conversationId" @open="emit('open-background-jobs')" />
        </div>
        <AskUserInputDialog
          v-if="pendingQuestion"
          :request="pendingQuestion"
          @submit="emit('ask-submit', $event)"
          @skip="emit('ask-skip')"
        />
        <InputArea
          v-else
          ref="inputAreaRef"
          :activeAgent="activeAgent"
          @send="handleSend"
          @cancel="emit('cancel')"
          @remove-agent="emit('remove-agent')"
          @compact="emit('compact')"
        />
      </div>
    </div>

    <SelectionActionPopover
      :visible="selectionPopover.visible"
      :x="selectionPopover.x"
      :y="selectionPopover.y"
      @quote="handleQuoteToInput"
      @branch="handleOpenBranch"
    />

    <BranchSidebar :conversation-id="conversationId" />
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


</style>
