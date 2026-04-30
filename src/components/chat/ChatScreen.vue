<script setup lang="ts">
import { nextTick, onMounted, ref } from 'vue';
import type {
  AgentMode,
  AskUserAnswerSubmission,
  ChatMessage,
  NeedsUserInputPayload,
  PendingUploadFile,
  ToolExecutionEntry,
} from '../../lib/chat-types';
import InputArea from '../layout/InputArea.vue';
import AskUserInputDialog from './AskUserInputDialog.vue';
import AssistantMessageBubble from './messages/AssistantMessageBubble.vue';
import CurrentTurnActivityRail from './messages/CurrentTurnActivityRail.vue';
import MarkdownRenderer from './MarkdownRenderer.vue';
import UserMessageBubble from './messages/UserMessageBubble.vue';

const props = defineProps<{
  messages: ChatMessage[];
  isGenerating: boolean;
  assistantResponse: string;
  assistantReasoning?: string;
  assistantTokenUsage?: number;
  currentTurnToolEntries: ToolExecutionEntry[];
  pendingQuestion?: NeedsUserInputPayload | null;
  pendingPermissionRequestId?: string | null;
  planMode?: boolean;
  agentMode?: AgentMode;
  pendingUploads?: PendingUploadFile[];
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'ask-submit', value: AskUserAnswerSubmission): void;
  (e: 'ask-skip'): void;
  (e: 'cancel'): void;
  (e: 'mode-change', mode: AgentMode): void;
  (e: 'upload-files', files: PendingUploadFile[]): void;
  (e: 'remove-upload', index: number): void;
}>();

const chatAreaRef = ref<HTMLElement | null>(null);
const reactionMap = ref<Record<number, 'up' | 'down' | undefined>>({});
const copiedMap = ref<Record<string, boolean>>({});
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
  scrollToBottom();
};

const retryFromAssistant = (assistantIndex: number) => {
  const prev = [...props.messages.slice(0, assistantIndex)].reverse().find((m) => m.role === 'user');
  if (!prev?.content?.trim()) return;
  emit('send', prev.content);
  scrollToBottom();
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

onMounted(() => {
  scrollToBottom();
});

const handleSend = (msg: string) => {
  emit('send', msg);
  scrollLastUserMessageToTop();
};

const handleUploadFiles = (files: PendingUploadFile[]) => {
  emit('upload-files', files);
};

const handleRemoveUpload = (index: number) => {
  emit('remove-upload', index);
};

const conversationTokenUsage = (index: number): number => {
  return props.messages.slice(0, index + 1).reduce((sum, m) => sum + (m.tokenUsage ?? 0), 0);
};

const estimateTokensFromContent = (content: string): number => {
  const normalized = content.replace(/\s+/g, ' ').trim();
  if (!normalized) return 0;
  return Math.max(1, Math.ceil(normalized.length / 4));
};

const streamingTokenUsage = (): number => {
  if (typeof props.assistantTokenUsage === 'number' && props.assistantTokenUsage > 0) {
    return props.assistantTokenUsage;
  }
  if (!props.isGenerating) {
    return 0;
  }
  return estimateTokensFromContent(props.assistantResponse);
};

const streamingConversationTokenUsage = (): number => {
  const base = props.messages.reduce((sum, m) => sum + (m.tokenUsage ?? 0), 0);
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

defineExpose({
  scrollToBottom,
  scrollLastUserMessageToTop,
});
</script>

<template>
  <div class="flex flex-col h-full w-full max-w-4xl mx-auto pt-14">
    <div class="flex-1 overflow-y-auto px-4 pb-4 custom-scrollbar" ref="chatAreaRef">
      <div class="w-full flex flex-col gap-6">
        <div
          v-for="(msg, index) in messages"
          :key="index"
          :data-role="msg.role"
          class="flex w-full group"
        >
          <UserMessageBubble
            v-if="msg.role === 'user'"
            :message="msg"
            :index="index"
            :copied="!!copiedMap[`user-${index}`]"
            :timeText="formatNowTime()"
            @retry="retryFromUser"
            @copy="copyText(msg.content, `user-${index}`)"
          />

          <AssistantMessageBubble
            v-else
            :message="msg"
            :index="index"
            :copied="!!copiedMap[`assistant-${index}`]"
            :conversationTokenUsage="conversationTokenUsage(index)"
            @copy="copyText(buildAssistantCopyText(msg), `assistant-${index}`)"
            @retry="retryFromAssistant"
            @react="setReaction($event.index, $event.value)"
          />
        </div>

        <div v-if="hasLiveAssistantTurn()" class="flex w-full justify-start group">
          <div class="flex gap-3.5 w-full max-w-[85%]">
            <div class="w-7 h-7 rounded-full flex items-center justify-center shrink-0 bg-[#f6f3ec] dark:bg-[#333] text-[#6f685a] mt-0.5 border border-[#e7e2d7] dark:border-[#444] text-[11px] font-medium">
              N
            </div>
            <div class="min-w-0 flex-1 text-[0.95rem] leading-relaxed break-words text-[#1a1a1a] dark:text-[#ececec]">
              <div class="flex items-center gap-2 mb-1">
                <p class="text-[11px] text-[#9b958a]">Nova</p>
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
              <details
                v-if="hasStreamingReasoning()"
                class="reasoning-panel mt-2"
                open
              >
                <summary>AI 思考过程</summary>
                <MarkdownRenderer :content="props.assistantReasoning || ''" />
              </details>
              <CurrentTurnActivityRail
                v-if="props.currentTurnToolEntries.length > 0 || !!liveWaitKind()"
                :entries="props.currentTurnToolEntries"
                :waitKind="liveWaitKind()"
              />
              <MarkdownRenderer v-if="streamingBodyText()" :content="assistantResponse" />
              <p
                v-else-if="props.currentTurnToolEntries.length > 0 || props.isGenerating"
                class="text-[13px] text-[#8e8678] dark:text-[#b2aa9c]"
              >
                正在处理你的请求...
              </p>
              <span
                v-if="isGenerating"
                class="inline-block w-1.5 h-[1em] bg-current ml-1 align-middle animate-pulse opacity-70"
              ></span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="w-full bg-transparent px-4 pt-4 pb-6">
      <div class="w-full max-w-[760px] mx-auto">
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

.token-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 9px;
  color: #A39E93;
  border: 1px solid rgba(229, 225, 213, 0.6);
  background: rgba(229, 225, 213, 0.2);
  padding: 3px 6px;
  border-radius: 6px;
  font-family: monospace;
  letter-spacing: 0.04em;
  font-variant-numeric: tabular-nums;
}

.dark .token-badge {
  color: #a09e99;
  border-color: #5a5549;
  background: rgba(60, 56, 48, 0.45);
}

.reasoning-panel {
  margin-bottom: 10px;
  border: 1px solid rgba(225, 218, 204, 0.9);
  background: rgba(249, 246, 239, 0.85);
  border-radius: 10px;
  padding: 8px 10px;
}

.reasoning-panel summary {
  cursor: pointer;
  font-size: 11px;
  color: #8a8478;
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

.reasoning-panel :deep(.markdown-body) {
  margin-top: 8px;
}

.dark .reasoning-panel {
  border-color: #4a443a;
  background: rgba(41, 38, 33, 0.92);
}

.dark .reasoning-panel summary {
  color: #b1ab9f;
}

</style>
