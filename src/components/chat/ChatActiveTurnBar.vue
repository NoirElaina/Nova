<script setup lang="ts">
import type {
  AssistantTranscriptSegment,
  ContextCompactSummary,
  ToolExecutionEntry,
} from '@/lib/chat-types';
import ContextCompactNotice from './messages/ContextCompactNotice.vue';
import AssistantTranscript from './messages/AssistantTranscript.vue';

defineProps<{
  contextCompacts?: ContextCompactSummary[];
  streamingSegments: AssistantTranscriptSegment[];
  currentTurnToolEntries: ToolExecutionEntry[];
  isGenerating: boolean;
  liveWaitKind: 'permission' | 'question' | null;
  liveStatusText: string;
  liveElapsedMs: number;
  streamingTokenUsage: number;
  streamingConversationTokenUsage: number;
  formatElapsedMs: (ms: number) => string;
}>();
</script>

<template>
  <div class="flex w-full justify-start group" data-role="assistant-live">
    <div class="w-full max-w-[85%]">
      <div class="min-w-0 flex-1 text-[0.95rem] leading-relaxed break-words text-[#1a1a1a] dark:text-[#ececec]">
        <ContextCompactNotice
          v-if="contextCompacts && contextCompacts.length > 0"
          :items="contextCompacts"
          compact
        />
        <AssistantTranscript
          v-if="streamingSegments.length > 0"
          :segments="streamingSegments"
          :entries="currentTurnToolEntries"
          live
        />
        <p
          v-else-if="isGenerating || !!liveWaitKind"
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
          v-if="liveElapsedMs > 0 || streamingTokenUsage > 0 || streamingConversationTokenUsage > 0"
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
            v-if="streamingTokenUsage > 0 || streamingConversationTokenUsage > 0"
            class="token-badge"
          >
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
              <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
              <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
            </svg>
            本次 {{ streamingTokenUsage }} · 会话 {{ streamingConversationTokenUsage }}
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
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
