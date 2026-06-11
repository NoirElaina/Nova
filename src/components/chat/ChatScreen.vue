<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import type {
  AgentMode,
  AskUserAnswerSubmission,
  ChatMessage,
  ContextCompactSummary,
  ContextUsage,
  NeedsUserInputPayload,
  PendingUploadFile,
  ToolExecutionEntry,
} from '../../lib/chat-types';
import type { LiveTurnStage } from '../../features/chat/controllers/chat-controller-types';
import InputArea from '../layout/InputArea.vue';
import AskUserInputDialog from './AskUserInputDialog.vue';
import AssistantMessageBubble from './messages/AssistantMessageBubble.vue';
import ContextCompactNotice from './messages/ContextCompactNotice.vue';
import CurrentTurnActivityRail from './messages/CurrentTurnActivityRail.vue';
import MarkdownRenderer from './MarkdownRenderer.vue';
import MessageTimelineNavigator from './MessageTimelineNavigator.vue';
import UserMessageBubble from './messages/UserMessageBubble.vue';

const props = defineProps<{
  messages: ChatMessage[];
  isGenerating: boolean;
  currentStage?: LiveTurnStage;
  assistantResponse: string;
  assistantReasoning?: string;
  assistantTokenUsage?: number;
  currentTurnToolEntries: ToolExecutionEntry[];
  pendingQuestion?: NeedsUserInputPayload | null;
  pendingPermissionRequestId?: string | null;
  planMode?: boolean;
  agentMode?: AgentMode;
  pendingUploads?: PendingUploadFile[];
  contextUsage?: ContextUsage;
  contextCompacts?: ContextCompactSummary[];
  contextTokens?: number;
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'save-user-edit', payload: { index: number; content: string }): void;
  (e: 'ask-submit', value: AskUserAnswerSubmission): void;
  (e: 'ask-skip'): void;
  (e: 'cancel'): void;
  (e: 'mode-change', mode: AgentMode): void;
  (e: 'upload-files', files: PendingUploadFile[]): void;
  (e: 'remove-upload', index: number): void;
}>();

const chatAreaRef = ref<HTMLElement | null>(null);
const liveAssistantRef = ref<HTMLElement | null>(null);
const reactionMap = ref<Record<number, 'up' | 'down' | undefined>>({});
const copiedMap = ref<Record<string, boolean>>({});
const showScrollToBottom = ref(false);
const activeUserMessageIndex = ref<number | null>(null);
const copyTimers: Record<string, ReturnType<typeof setTimeout> | undefined> = {};

const formatNowTime = () => {
  const now = new Date();
  const hh = String(now.getHours()).padStart(2, '0');
  const mm = String(now.getMinutes()).padStart(2, '0');
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
  const sections = [];
  if (message.reasoning?.trim()) {
    sections.push(`AI 思考过程\n${message.reasoning.trim()}`);
  }
  if (message.content?.trim()) {
    sections.push(message.content.trim());
  }
  return sections.join('\n\n');
};

const scrollToBottom = async () => {
  await nextTick();
  if (chatAreaRef.value) {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
  }
  updateScrollToBottomVisibility();
};

const scrollLastUserMessageToTop = async () => {
  await nextTick();
  if (!chatAreaRef.value) return;
  const rows = chatAreaRef.value.querySelectorAll<HTMLElement>('[data-role="user"]');
  const last = rows[rows.length - 1];
  if (last) {
    last.scrollIntoView({ block: 'start', behavior: 'smooth' });
  } else {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
  }
};

const scrollLastUserMessageToBottom = async () => {
  await nextTick();
  if (!chatAreaRef.value) return;
  const rows = chatAreaRef.value.querySelectorAll<HTMLElement>('[data-role="user"]');
  const last = rows[rows.length - 1];
  if (last) {
    last.scrollIntoView({ block: 'end', behavior: 'smooth' });
  } else {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
  }
  updateScrollToBottomVisibility();
};

const scrollLiveAssistantIntoView = async () => {
  await nextTick();
  const target = liveAssistantRef.value;
  if (target) {
    target.scrollIntoView({ block: 'start', behavior: 'smooth' });
  } else if (chatAreaRef.value) {
    chatAreaRef.value.scrollTop = chatAreaRef.value.scrollHeight;
  }
  updateScrollToBottomVisibility();
};

const updateScrollToBottomVisibility = () => {
  if (!chatAreaRef.value) {
    showScrollToBottom.value = false;
    return;
  }
  const distanceFromBottom =
    chatAreaRef.value.scrollHeight - chatAreaRef.value.clientHeight - chatAreaRef.value.scrollTop;
  showScrollToBottom.value = distanceFromBottom > 120;
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
    activeUserMessageIndex.value = null;
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
  chatAreaRef.value?.scrollTo({
    top: chatAreaRef.value.scrollHeight,
    behavior: 'smooth',
  });
};

const scrollToMessageIndex = async (index: number) => {
  await nextTick();
  const container = chatAreaRef.value;
  const target = container?.querySelector<HTMLElement>(`[data-message-index="${index}"]`);
  if (!target) return;
  activeUserMessageIndex.value = index;
  target.scrollIntoView({ block: 'start', behavior: 'smooth' });
};

onMounted(() => {
  void scrollToBottom();
  void nextTick(updateActiveUserMessage);
});

watch(
  () => [
    props.messages.length,
    props.assistantResponse,
    props.assistantReasoning,
    props.currentTurnToolEntries.length,
    props.isGenerating,
  ],
    async () => {
      await nextTick();
      updateScrollToBottomVisibility();
      updateActiveUserMessage();
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
  return props.messages.slice(0, index + 1).reduce((sum, m) => {
    const costTotal = (m.cost?.inputTokens ?? 0) + (m.cost?.outputTokens ?? 0);
    return sum + (costTotal > 0 ? costTotal : (m.tokenUsage ?? 0));
  }, 0);
};

const estimateTokensFromContent = (content: string): number => {
  const normalized = content.replace(/\s+/g, ' ').trim();
  if (!normalized) return 0;
  return Math.max(1, Math.ceil(normalized.length / 4));
};

const streamingTokenUsage = (): number => {
  const outputTokens =
    typeof props.assistantTokenUsage === 'number' && props.assistantTokenUsage > 0
      ? props.assistantTokenUsage
      : props.isGenerating
        ? estimateTokensFromContent(props.assistantResponse)
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
  const base = props.messages.reduce((sum, m) => {
    const costTotal = (m.cost?.inputTokens ?? 0) + (m.cost?.outputTokens ?? 0);
    return sum + (costTotal > 0 ? costTotal : (m.tokenUsage ?? 0));
  }, 0);
  return base + streamingTokenUsage();
};

const hasStreamingReasoning = () => !!props.assistantReasoning?.trim();
const streamingBodyText = () => props.assistantResponse.trim();
const hasLiveAssistantTurn = () =>
  props.isGenerating ||
  hasStreamingReasoning() ||
  streamingBodyText().length > 0 ||
  props.currentTurnToolEntries.length > 0;
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
    >
      <div class="w-full flex flex-col gap-6">
        <div
          v-for="(msg, index) in messages"
          :key="index"
          :data-role="msg.role"
          :data-message-index="index"
          class="flex w-full group"
        >
          <UserMessageBubble
            v-if="msg.role === 'user'"
            :message="msg"
            :index="index"
            :copied="!!copiedMap[`user-${index}`]"
            :timeText="formatNowTime()"
            @retry="retryFromUser"
            @save-edit="emit('save-user-edit', $event)"
            @copy="copyText(msg.content, `user-${index}`)"
          />

          <AssistantMessageBubble
            v-else
            :message="msg"
            :index="index"
            :copied="!!copiedMap[`assistant-${index}`]"
            :conversationTokenUsage="conversationTokenUsage(index)"
            :reaction="reactionMap[index]"
            @copy="copyText(buildAssistantCopyText(msg), `assistant-${index}`)"
            @retry="retryFromAssistant"
            @react="setReaction($event.index, $event.value)"
          />
        </div>

        <div
          v-if="hasLiveAssistantTurn()"
          ref="liveAssistantRef"
          class="flex w-full justify-start group"
        >
          <div class="w-full max-w-[85%]">
            <div class="min-w-0 flex-1 text-[0.95rem] leading-relaxed break-words text-[#1a1a1a] dark:text-[#ececec]">
              <details
                v-if="hasStreamingReasoning()"
                class="reasoning-panel mt-2"
                open
              >
                <summary>AI 思考过程</summary>
                <MarkdownRenderer :content="props.assistantReasoning || ''" />
              </details>
              <ContextCompactNotice
                v-if="props.contextCompacts && props.contextCompacts.length > 0"
                :items="props.contextCompacts"
                compact
              />
              <CurrentTurnActivityRail
                v-if="props.currentTurnToolEntries.length > 0 || !!liveWaitKind()"
                :entries="props.currentTurnToolEntries"
                :waitKind="liveWaitKind()"
              />
              <MarkdownRenderer v-if="streamingBodyText()" :content="assistantResponse" />
              <p
                v-else-if="props.currentTurnToolEntries.length > 0 || props.isGenerating"
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
                v-if="streamingTokenUsage() > 0 || streamingConversationTokenUsage() > 0"
                class="token-badge mt-2"
              >
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                  <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
                  <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
                  <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
                </svg>
                本次 {{ streamingTokenUsage() }} · 会话 {{ streamingConversationTokenUsage() }}
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
          @send="handleSend"
          @cancel="emit('cancel')"
          @mode-change="emit('mode-change', $event)"
          @upload-files="handleUploadFiles"
          @remove-upload="handleRemoveUpload"
        />
      </div>
      <div class="text-center text-[0.7rem] text-muted-foreground mt-2">
        Nova can make mistakes. Please verify important information.
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-scroll-area {
  position: relative;
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

.reasoning-panel {
  margin-bottom: 10px;
  border: 1px solid #e5e7eb;
  background: #fafafa;
  border-radius: 10px;
  padding: 8px 10px;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.035);
}

.reasoning-panel summary {
  cursor: pointer;
  font-size: 11px;
  color: #64748b;
  user-select: none;
  list-style: none;
}

.reasoning-panel summary::-webkit-details-marker {
  display: none;
}

.reasoning-panel summary::before {
  content: "▸";
  display: inline-block;
  margin-right: 6px;
  transition: transform 0.15s ease;
}

.reasoning-panel[open] summary::before {
  transform: rotate(90deg);
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

.reasoning-panel :deep(.markdown-body) {
  margin-top: 8px;
}

.dark .reasoning-panel {
  border-color: #3f4652;
  background: #1f2937;
}

.dark .reasoning-panel summary {
  color: #cbd5e1;
}

</style>
