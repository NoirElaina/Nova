<script setup lang="ts">
import { Button } from '@/components/ui/button';
import MarkdownRenderer from '../MarkdownRenderer.vue';
import type { ChatMessage } from '../../../lib/chat-types';

defineProps<{
  message: ChatMessage;
  index: number;
  copied: boolean;
  conversationTokenUsage: number;
}>();

const emit = defineEmits<{
  (e: 'copy', index: number): void;
  (e: 'retry', index: number): void;
  (e: 'react', payload: { index: number; value: 'up' | 'down' }): void;
}>();
</script>

<template>
  <div class="flex gap-3.5 w-full max-w-[85%]">
    <div class="w-7 h-7 rounded-full flex items-center justify-center shrink-0 bg-[#f6f3ec] dark:bg-[#333] text-[#6f685a] mt-0.5 border border-[#e7e2d7] dark:border-[#444] text-[11px] font-medium">
      N
    </div>
    <div class="min-w-0 flex-1 text-[0.95rem] leading-relaxed break-words text-[#1a1a1a] dark:text-[#ececec]">
      <div class="flex items-center gap-2 mb-1">
        <p class="text-[11px] text-[#9b958a]">Nova</p>
        <span
          v-if="(message.tokenUsage && message.tokenUsage > 0) || (conversationTokenUsage && conversationTokenUsage > 0)"
          class="token-badge"
        >
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
            <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
            <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
          </svg>
          本次 {{ message.tokenUsage ?? 0 }} · 会话 {{ conversationTokenUsage }}
        </span>
      </div>
      <details
        v-if="message.reasoning?.trim()"
        class="reasoning-panel"
      >
        <summary>AI 思考过程</summary>
        <MarkdownRenderer :content="message.reasoning" />
      </details>
      <MarkdownRenderer :content="message.content" />
      <div class="msg-toolbar">
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" :class="{ 'is-copied': copied }" aria-label="Copy assistant message" @click="emit('copy', index)">
          <svg v-if="!copied" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
          <svg v-else width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" aria-label="Thumbs up" @click="emit('react', { index, value: 'up' })">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3H14z"/><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" aria-label="Thumbs down" @click="emit('react', { index, value: 'down' })">
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
  color: #bbb6ae;
  cursor: pointer;
  transition: color 0.15s, background 0.15s;
}

.msg-icon-btn:hover {
  color: #6b6456;
  background: #f0ede7;
}

.msg-icon-btn.is-copied {
  color: #4a7c59;
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
