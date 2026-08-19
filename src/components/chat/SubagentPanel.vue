<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import {
  initSubagentEvents,
  subagentsFor,
  type SubagentEntry,
} from '../../features/chat/services/subagents';

const props = defineProps<{
  conversationId?: string | null;
}>();

onMounted(() => {
  void initSubagentEvents();
});

const panelOpen = ref(false);
const expandedCards = ref(new Set<string>());

const entries = computed<SubagentEntry[]>(() => subagentsFor(props.conversationId));
const runningCount = computed(() => entries.value.filter((e) => e.phase === 'running').length);
const doneCount = computed(() => entries.value.filter((e) => e.phase !== 'running').length);

// 会话切换或全部结束时自动收起按钮上下文，但保留已完成的记录供查看。
watch(
  () => props.conversationId,
  () => {
    expandedCards.value.clear();
  },
);

const toggleCard = (subId: string) => {
  const next = new Set(expandedCards.value);
  if (next.has(subId)) {
    next.delete(subId);
  } else {
    next.add(subId);
  }
  expandedCards.value = next;
  void nextTick(scrollLogToBottom);
};

const formatElapsed = (ms?: number) => {
  if (typeof ms !== 'number') return '';
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m${seconds % 60}s`;
};

const phaseLabel = (entry: SubagentEntry) =>
  entry.phase === 'running' ? '运行中' : entry.phase === 'error' ? '出错' : '完成';

const logContainerRefs = ref(new Map<string, HTMLElement>());
const setLogRef = (subId: string, el: unknown) => {
  if (el instanceof HTMLElement) {
    logContainerRefs.value.set(subId, el);
  } else {
    logContainerRefs.value.delete(subId);
  }
};

const scrollLogToBottom = () => {
  for (const el of logContainerRefs.value.values()) {
    el.scrollTop = el.scrollHeight;
  }
};

watch(
  () => entries.value.map((e) => e.lines.length).join(','),
  () => void nextTick(scrollLogToBottom),
);

// 抽屉高度自适应：顶部避开全局 header（56px），底部留边，由 CSS calc 提供。
const drawerTop = '56px';
</script>

<template>
  <Teleport to="body">
    <!-- 浮动按钮：当前会话存在子代理记录时显示 -->
    <button
      v-if="entries.length > 0"
      type="button"
      class="subagent-fab"
      :class="{ 'subagent-fab--active': panelOpen }"
      :title="panelOpen ? '收起子代理面板' : '查看子代理'"
      @click="panelOpen = !panelOpen"
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <circle cx="11" cy="11" r="7" />
        <path d="m21 21-4.3-4.3" />
      </svg>
      <span>子代理</span>
      <span v-if="runningCount > 0" class="subagent-fab__badge subagent-fab__badge--running">
        {{ runningCount }} 运行
      </span>
      <span v-else class="subagent-fab__badge">{{ doneCount }} 完成</span>
    </button>

    <!-- 右侧抽屉 -->
    <Transition name="subagent-drawer">
      <div v-if="panelOpen && entries.length > 0" class="subagent-drawer" :style="{ top: drawerTop }">
        <div class="subagent-drawer__header">
          <div class="subagent-drawer__title">子代理</div>
          <div class="subagent-drawer__meta">
            {{ runningCount }} 运行 · {{ doneCount }} 完成
          </div>
          <button
            type="button"
            class="subagent-drawer__close"
            aria-label="关闭子代理面板"
            @click="panelOpen = false"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>

        <div class="subagent-drawer__body">
          <div v-if="entries.length === 0" class="subagent-empty">当前会话暂无子代理记录</div>

          <div v-for="entry in entries" :key="entry.subId" class="subagent-card">
            <button type="button" class="subagent-card__head" @click="toggleCard(entry.subId)">
              <span
                class="subagent-card__dot"
                :class="{
                  'subagent-card__dot--running': entry.phase === 'running',
                  'subagent-card__dot--error': entry.phase === 'error',
                }"
              />
              <span class="subagent-card__status">{{ phaseLabel(entry) }}</span>
              <span class="subagent-card__elapsed">{{ formatElapsed(entry.elapsedMs) }}</span>
              <svg
                class="subagent-card__chevron"
                :class="{ 'subagent-card__chevron--open': expandedCards.has(entry.subId) }"
                width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
              >
                <path d="m9 18 6-6-6-6" />
              </svg>
            </button>

            <div class="subagent-card__task" :title="entry.task">{{ entry.task }}</div>

            <div v-if="entry.detail" class="subagent-card__detail">{{ entry.detail }}</div>

            <div v-if="entry.reportPreview && entry.phase !== 'running'" class="subagent-card__report">
              <div class="subagent-card__report-label">报告预览</div>
              <div class="subagent-card__report-text">{{ entry.reportPreview }}</div>
            </div>

            <div v-show="expandedCards.has(entry.subId)" class="subagent-card__log-wrap">
              <div :ref="(el) => setLogRef(entry.subId, el)" class="subagent-card__log">
                <div v-if="entry.lines.length === 0" class="subagent-card__log-empty">
                  等待子代理输出…
                </div>
                <template v-for="(line, index) in entry.lines" :key="index">
                  <div v-if="line.kind === 'tool'" class="subagent-log-line subagent-log-line--tool">
                    ⚙ {{ line.toolName }}
                  </div>
                  <div
                    v-else-if="line.kind === 'tool-result'"
                    class="subagent-log-line subagent-log-line--result"
                    :class="{ 'subagent-log-line--error': line.isError }"
                  >
                    <span class="subagent-log-line__tool">↳ {{ line.toolName }}</span>
                    <span class="subagent-log-line__text">{{ line.text }}</span>
                  </div>
                  <div v-else-if="line.kind === 'reasoning'" class="subagent-log-line subagent-log-line--reasoning">
                    {{ line.text }}
                  </div>
                  <div v-else class="subagent-log-line subagent-log-line--text">{{ line.text }}</div>
                </template>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.subagent-fab {
  position: fixed;
  right: 20px;
  bottom: 150px;
  z-index: 40;
  display: inline-flex;
  align-items: center;
  gap: 7px;
  border: 1px solid rgba(226, 216, 201, 0.9);
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.92);
  backdrop-filter: blur(6px);
  color: #4f473c;
  cursor: pointer;
  font-size: 13px;
  padding: 8px 12px;
  box-shadow: 0 6px 18px rgba(55, 45, 31, 0.12);
  transition: background 160ms ease, box-shadow 160ms ease, color 160ms ease;
}

.subagent-fab:hover {
  background: #fff;
  box-shadow: 0 8px 22px rgba(55, 45, 31, 0.18);
}

.subagent-fab--active {
  background: #1f1f1d;
  color: #fffaf0;
}

.subagent-fab__badge {
  border-radius: 999px;
  background: rgba(31, 31, 29, 0.08);
  color: inherit;
  font-size: 11px;
  padding: 2px 8px;
}

.subagent-fab__badge--running {
  background: rgba(34, 197, 94, 0.16);
  color: #15803d;
}

.dark .subagent-fab {
  border-color: #46413a;
  background: rgba(37, 37, 37, 0.92);
  color: #d6cec2;
}

.dark .subagent-fab--active {
  background: #eee4d6;
  color: #1f1f1d;
}

.dark .subagent-fab__badge {
  background: rgba(255, 255, 255, 0.08);
}

.dark .subagent-fab__badge--running {
  background: rgba(34, 197, 94, 0.2);
  color: #4ade80;
}

.subagent-drawer {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 45;
  display: flex;
  flex-direction: column;
  width: 400px;
  max-width: calc(100vw - 32px);
  height: calc(100vh - 56px - 32px);
  border: 1px solid rgba(226, 216, 201, 0.9);
  border-radius: 16px;
  background: rgba(255, 255, 255, 0.97);
  backdrop-filter: blur(10px);
  box-shadow: 0 18px 48px rgba(55, 45, 31, 0.2);
  overflow: hidden;
}

.dark .subagent-drawer {
  border-color: #3a362f;
  background: rgba(30, 30, 30, 0.97);
}

.subagent-drawer__header {
  display: flex;
  align-items: center;
  gap: 10px;
  border-bottom: 1px solid rgba(226, 216, 201, 0.7);
  padding: 14px 16px;
}

.dark .subagent-drawer__header {
  border-bottom-color: #38342e;
}

.subagent-drawer__title {
  font-size: 15px;
  font-weight: 700;
  color: #1a1a1a;
}

.dark .subagent-drawer__title {
  color: #ececec;
}

.subagent-drawer__meta {
  flex: 1;
  font-size: 12px;
  color: #8b816f;
}

.subagent-drawer__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: #8b816f;
  cursor: pointer;
  padding: 6px;
}

.subagent-drawer__close:hover {
  background: rgba(0, 0, 0, 0.05);
  color: #1a1a1a;
}

.dark .subagent-drawer__close:hover {
  background: rgba(255, 255, 255, 0.07);
  color: #ececec;
}

.subagent-drawer__body {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.subagent-empty {
  color: #8b816f;
  font-size: 13px;
  text-align: center;
  padding: 24px 0;
}

.subagent-card {
  border: 1px solid rgba(226, 216, 201, 0.8);
  border-radius: 12px;
  background: rgba(249, 248, 245, 0.8);
  padding: 10px 12px;
}

.dark .subagent-card {
  border-color: #38342e;
  background: rgba(255, 255, 255, 0.03);
}

.subagent-card__head {
  display: flex;
  align-items: center;
  gap: 8px;
  border: 0;
  background: transparent;
  cursor: pointer;
  padding: 0;
  width: 100%;
  text-align: left;
}

.subagent-card__dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: #9ca3af;
  flex-shrink: 0;
}

.subagent-card__dot--running {
  background: #22c55e;
  animation: subagent-pulse 1.4s ease-in-out infinite;
}

.subagent-card__dot--error {
  background: #f43f5e;
}

@keyframes subagent-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}

.subagent-card__status {
  font-size: 12px;
  font-weight: 600;
  color: #4f473c;
}

.dark .subagent-card__status {
  color: #d6cec2;
}

.subagent-card__elapsed {
  flex: 1;
  font-size: 11px;
  color: #8b816f;
  font-variant-numeric: tabular-nums;
}

.subagent-card__chevron {
  color: #8b816f;
  transition: transform 160ms ease;
}

.subagent-card__chevron--open {
  transform: rotate(90deg);
}

.subagent-card__task {
  margin-top: 6px;
  font-size: 12.5px;
  line-height: 1.5;
  color: #1a1a1a;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.dark .subagent-card__task {
  color: #d6cec2;
}

.subagent-card__detail {
  margin-top: 6px;
  font-size: 12px;
  color: #be123c;
  word-break: break-all;
}

.subagent-card__report {
  margin-top: 8px;
  border-left: 3px solid rgba(34, 197, 94, 0.5);
  padding-left: 8px;
}

.subagent-card__report-label {
  font-size: 10px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #8b816f;
  margin-bottom: 3px;
}

.subagent-card__report-text {
  font-size: 12px;
  line-height: 1.55;
  color: #1a1a1a;
  white-space: pre-wrap;
  word-break: break-word;
  display: -webkit-box;
  -webkit-line-clamp: 6;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.dark .subagent-card__report-text {
  color: #d6cec2;
}

.subagent-card__log-wrap {
  margin-top: 8px;
}

.subagent-card__log {
  max-height: 260px;
  overflow-y: auto;
  border-radius: 8px;
  background: rgba(31, 29, 26, 0.05);
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dark .subagent-card__log {
  background: rgba(0, 0, 0, 0.28);
}

.subagent-card__log-empty {
  font-size: 12px;
  color: #8b816f;
}

.subagent-log-line {
  font-size: 12px;
  line-height: 1.5;
  word-break: break-word;
  white-space: pre-wrap;
}

.subagent-log-line--tool {
  color: #2563eb;
  font-weight: 600;
}

.dark .subagent-log-line--tool {
  color: #60a5fa;
}

.subagent-log-line--result {
  color: #6b7280;
}

.subagent-log-line--result .subagent-log-line__tool {
  display: block;
  font-weight: 600;
  color: #4f473c;
}

.dark .subagent-log-line--result {
  color: #9ca3af;
}

.dark .subagent-log-line--result .subagent-log-line__tool {
  color: #b9b0a2;
}

.subagent-log-line--error {
  color: #be123c;
}

.subagent-log-line--reasoning {
  color: #a8a29e;
  font-style: italic;
}

.subagent-log-line--text {
  color: #1a1a1a;
}

.dark .subagent-log-line--text {
  color: #e5e5e5;
}

.subagent-drawer-enter-active, .subagent-drawer-leave-active {
  transition: opacity 180ms ease, transform 180ms ease;
}

.subagent-drawer-enter-from, .subagent-drawer-leave-to {
  opacity: 0;
  transform: translateX(24px);
}
</style>
