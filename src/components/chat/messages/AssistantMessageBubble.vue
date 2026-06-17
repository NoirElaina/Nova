<script setup lang="ts">
import { ref, computed } from 'vue';
import { Button } from '@/components/ui/button';
import MarkdownRenderer from '../MarkdownRenderer.vue';
import type { ChatMessage } from '../../../lib/chat-types';
import ContextCompactNotice from './ContextCompactNotice.vue';
import TurnActivitySummaryCard from './TurnActivitySummaryCard.vue';

const props = defineProps<{
  message: ChatMessage;
  index: number;
  copied: boolean;
  conversationTokenUsage: number;
  reaction?: 'up' | 'down';
}>();

const emit = defineEmits<{
  (e: 'copy', index: number): void;
  (e: 'retry', index: number): void;
  (e: 'react', payload: { index: number; value: 'up' | 'down' }): void;
}>();

const animatingReaction = ref<'up' | 'down' | null>(null);
const reasoningOpen = ref(false);

const formatUsd = (value?: string) => {
  const amount = Number.parseFloat(value ?? '');
  if (!Number.isFinite(amount) || amount <= 0) return '';
  if (amount < 0.0001) return `$${amount.toPrecision(2)}`;
  if (amount < 0.01) return `$${amount.toFixed(5)}`;
  return `$${amount.toFixed(4)}`;
};

// 计算思考内容的字符数，显示在折叠 header 里
const reasoningChars = computed(() => {
  const len = props.message.reasoning?.trim().length ?? 0;
  if (len === 0) return '';
  if (len < 1000) return `${len} 字`;
  return `${(len / 1000).toFixed(1)}k 字`;
});

const triggerReaction = (value: 'up' | 'down') => {
  animatingReaction.value = value;
  emit('react', { index: props.index, value });
  window.setTimeout(() => {
    if (animatingReaction.value === value) {
      animatingReaction.value = null;
    }
  }, 320);
};
</script>

<template>
  <div class="w-full max-w-[85%]">
    <div class="min-w-0 flex-1 text-[0.95rem] leading-relaxed break-words text-[#1a1a1a] dark:text-[#ececec]">
      <div
        v-if="message.reasoning?.trim()"
        class="reasoning-panel"
        :class="{ 'reasoning-panel--open': reasoningOpen }"
      >
        <button class="reasoning-summary" @click="reasoningOpen = !reasoningOpen">
          <span class="reasoning-arrow">▸</span>
          <span class="reasoning-label">AI 思考过程</span>
          <span class="reasoning-meta">{{ reasoningChars }}</span>
        </button>
        <div class="reasoning-body">
          <div class="reasoning-scroll">
            <MarkdownRenderer :content="message.reasoning" />
          </div>
        </div>
      </div>
      <TurnActivitySummaryCard
        v-if="message.cost?.toolSummary && message.cost.toolSummary.totalCalls > 0"
        :summary="message.cost.toolSummary"
      />
      <ContextCompactNotice
        v-if="message.cost && message.cost.contextCompacts.length > 0"
        :items="message.cost.contextCompacts"
      />
      <MarkdownRenderer :content="message.content" />
      <div
        v-if="((message.cost?.inputTokens ?? 0) + (message.cost?.outputTokens ?? 0) > 0) || (message.tokenUsage && message.tokenUsage > 0) || (conversationTokenUsage && conversationTokenUsage > 0)"
        class="token-badge mt-2"
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
          <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
        </svg>
        本次 {{ ((message.cost?.inputTokens ?? 0) + (message.cost?.outputTokens ?? 0)) || (message.tokenUsage ?? 0) }} · 会话 {{ conversationTokenUsage }}<template v-if="formatUsd(message.cost?.totalCostUsd)"> · {{ formatUsd(message.cost?.totalCostUsd) }}</template>
      </div>
      <div class="msg-toolbar">
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" :class="{ 'is-copied': copied }" aria-label="Copy assistant message" @click="emit('copy', index)">
          <svg v-if="!copied" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
          <svg v-else width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          class="msg-icon-btn reaction-btn"
          :class="{
            'is-active-up': props.reaction === 'up',
            'is-animating': animatingReaction === 'up',
          }"
          aria-label="Thumbs up"
          @click="triggerReaction('up')"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3H14z"/><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/></svg>
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          class="msg-icon-btn reaction-btn"
          :class="{
            'is-active-down': props.reaction === 'down',
            'is-animating': animatingReaction === 'down',
          }"
          aria-label="Thumbs down"
          @click="triggerReaction('down')"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M10 15v4a3 3 0 0 0 3 3l4-9V2H5.72a2 2 0 0 0-2 1.7l-1.38 9a2 2 0 0 0 2 2.3H10z"/><path d="M17 2h2.67A2.31 2.31 0 0 1 22 4v7a2.31 2.31 0 0 1-2.33 2H17"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" aria-label="Retry" @click="emit('retry', index)">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
        </Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.msg-toolbar {
  display: flex;
  align-items: center;
  gap: 1px;
  margin-top: 4px;
  padding: 0 1px;
}

.msg-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  border-radius: 5px;
  color: #94a3b8;
  cursor: pointer;
  transition: color 0.15s, background 0.15s;
}

.msg-icon-btn:hover {
  color: #334155;
  background: #f1f5f9;
}

.msg-icon-btn.is-copied {
  color: #4a7c59;
}

.reaction-btn {
  transform-origin: center;
}

.reaction-btn.is-active-up {
  color: #4f8a62;
  background: rgba(112, 177, 132, 0.12);
}

.reaction-btn.is-active-down {
  color: #a3685c;
  background: rgba(196, 122, 104, 0.12);
}

.reaction-btn.is-animating {
  animation: reaction-pop 0.32s ease;
}

.token-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 9px;
  color: #047857;
  border: 1px solid #a7f3d0;
  background: #ecfdf5;
  margin-top: 8px;
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
  background: #f7f7f7;
  border-radius: 10px;
  overflow: hidden;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.035);
}

/* ---- 折叠栏 header ---- */
.reasoning-summary {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 8px 10px;
  background: transparent;
  border: none;
  cursor: pointer;
  font-size: 11px;
  color: #64748b;
  user-select: none;
  text-align: left;
}

.reasoning-summary:hover {
  background: rgba(0, 0, 0, 0.03);
}

.reasoning-arrow {
  display: inline-block;
  transition: transform 0.18s ease;
  flex-shrink: 0;
}

.reasoning-panel--open .reasoning-arrow {
  transform: rotate(90deg);
}

.reasoning-label {
  font-weight: 500;
}

.reasoning-meta {
  margin-left: auto;
  font-size: 10px;
  opacity: 0.6;
  font-family: monospace;
}

/* ---- 展开体（带 max-height 滚动）---- */
.reasoning-body {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.22s ease;
}

.reasoning-panel--open .reasoning-body {
  grid-template-rows: 1fr;
}

.reasoning-scroll {
  overflow: hidden;
  /* 展开后内部区域最大高度 260px，超出滚动 */
}

.reasoning-panel--open .reasoning-scroll {
  overflow-y: auto;
  max-height: 260px;
  padding: 0 10px 10px;
}

.reasoning-panel--open .reasoning-scroll::-webkit-scrollbar {
  width: 4px;
}

.reasoning-panel--open .reasoning-scroll::-webkit-scrollbar-track {
  background: transparent;
}

.reasoning-panel--open .reasoning-scroll::-webkit-scrollbar-thumb {
  background: rgba(100, 116, 139, 0.3);
  border-radius: 4px;
}

.reasoning-panel :deep(.markdown-body) {
  margin-top: 4px;
}

.dark .reasoning-panel {
  border-color: #3f4652;
  background: #1f2937;
}

.dark .reasoning-summary {
  color: #cbd5e1;
}

.dark .reasoning-summary:hover {
  background: rgba(255, 255, 255, 0.04);
}

.dark .reasoning-panel--open .reasoning-scroll::-webkit-scrollbar-thumb {
  background: rgba(148, 163, 184, 0.28);
}

.dark .reaction-btn.is-active-up {
  color: #8fd2a4;
  background: rgba(85, 145, 104, 0.24);
}

.dark .reaction-btn.is-active-down {
  color: #e2a297;
  background: rgba(125, 73, 63, 0.24);
}

@keyframes reaction-pop {
  0% {
    transform: scale(1);
  }
  35% {
    transform: scale(1.28);
  }
  70% {
    transform: scale(0.94);
  }
  100% {
    transform: scale(1);
  }
}

</style>
