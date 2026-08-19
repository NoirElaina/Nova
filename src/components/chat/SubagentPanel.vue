<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import {
  initSubagentEvents,
  panelOpen,
  subagentsFor,
  type SubagentEntry,
} from '../../features/chat/services/subagents';

const props = defineProps<{
  conversationId?: string | null;
}>();

onMounted(() => {
  void initSubagentEvents();
});

// 两级视图：null = 卡片列表；非 null = 该子代理的详情页。
const selectedSubId = ref<string | null>(null);

const entries = computed<SubagentEntry[]>(() => subagentsFor(props.conversationId));
const runningCount = computed(() => entries.value.filter((e) => e.phase === 'running').length);
const doneCount = computed(() => entries.value.filter((e) => e.phase !== 'running').length);

const selectedEntry = computed(
  () => entries.value.find((e) => e.subId === selectedSubId.value) ?? null,
);

// 会话切换时回列表视图；条目被清理（如删除会话）时同样回退。
watch(
  () => props.conversationId,
  () => {
    selectedSubId.value = null;
  },
);

watch(selectedEntry, (entry) => {
  if (!entry) selectedSubId.value = null;
});

const openDetail = (subId: string) => {
  selectedSubId.value = subId;
  void nextTick(scrollLogToBottom);
};

const backToList = () => {
  selectedSubId.value = null;
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

// 详情页日志容器：流式输出时自动滚到底部（仅详情页存在，单 ref 即可）。
const logEl = ref<HTMLElement | null>(null);

const scrollLogToBottom = () => {
  const el = logEl.value;
  if (el) el.scrollTop = el.scrollHeight;
};

watch(
  () => selectedEntry.value?.lines.length,
  () => void nextTick(scrollLogToBottom),
);

// 抽屉高度自适应：顶部避开全局 header（56px），底部留边，由 CSS calc 提供。
const drawerTop = '56px';
</script>

<template>
  <Teleport to="body">
    <!-- 右侧抽屉（按钮在 InputArea 状态行内，通过共享 panelOpen 控制） -->
    <Transition name="subagent-drawer">
      <div v-if="panelOpen && entries.length > 0" class="subagent-drawer" :style="{ top: drawerTop }">
        <!-- 详情页 -->
        <template v-if="selectedEntry">
          <div class="subagent-drawer__header">
            <button
              type="button"
              class="subagent-drawer__back"
              aria-label="返回子代理列表"
              @click="backToList"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="m15 18-6-6 6-6" />
              </svg>
            </button>
            <div class="subagent-drawer__title">子代理详情</div>
            <div class="subagent-drawer__meta">{{ formatElapsed(selectedEntry.elapsedMs) }}</div>
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

          <div class="subagent-detail">
            <div class="subagent-detail__status-row">
              <span
                class="subagent-card__dot"
                :class="{
                  'subagent-card__dot--running': selectedEntry.phase === 'running',
                  'subagent-card__dot--error': selectedEntry.phase === 'error',
                }"
              />
              <span class="subagent-detail__phase">{{ phaseLabel(selectedEntry) }}</span>
              <span class="subagent-detail__started">
                {{ new Date(selectedEntry.startedAt).toLocaleTimeString() }} 开始
              </span>
            </div>

            <div class="subagent-detail__section">
              <div class="subagent-detail__label">任务</div>
              <div class="subagent-detail__task">{{ selectedEntry.task }}</div>
            </div>

            <div v-if="selectedEntry.detail" class="subagent-detail__section">
              <div class="subagent-detail__label">状态信息</div>
              <div class="subagent-detail__error-text">{{ selectedEntry.detail }}</div>
            </div>

            <div v-if="selectedEntry.reportPreview" class="subagent-detail__section">
              <div class="subagent-detail__label">报告</div>
              <div class="subagent-detail__report">{{ selectedEntry.reportPreview }}</div>
            </div>

            <div class="subagent-detail__section subagent-detail__section--log">
              <div class="subagent-detail__label">运行日志</div>
              <div ref="logEl" class="subagent-detail__log">
                <div v-if="selectedEntry.lines.length === 0" class="subagent-card__log-empty">
                  等待子代理输出…
                </div>
                <template v-for="(line, index) in selectedEntry.lines" :key="index">
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
        </template>

        <!-- 列表页 -->
        <template v-else>
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

            <button
              v-for="entry in entries"
              :key="entry.subId"
              type="button"
              class="subagent-card"
              @click="openDetail(entry.subId)"
            >
              <span class="subagent-card__head">
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
                  width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                  stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
                >
                  <path d="m9 18 6-6-6-6" />
                </svg>
              </span>

              <span class="subagent-card__task" :title="entry.task">{{ entry.task }}</span>

              <span v-if="entry.reportPreview && entry.phase !== 'running'" class="subagent-card__report">
                <span class="subagent-card__report-label">报告预览</span>
                <span class="subagent-card__report-text">{{ entry.reportPreview }}</span>
              </span>
            </button>
          </div>
        </template>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
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

.subagent-drawer__back,
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

.subagent-drawer__back:hover,
.subagent-drawer__close:hover {
  background: rgba(0, 0, 0, 0.05);
  color: #1a1a1a;
}

.dark .subagent-drawer__back:hover,
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
  display: block;
  border: 1px solid rgba(226, 216, 201, 0.8);
  border-radius: 12px;
  background: rgba(249, 248, 245, 0.8);
  padding: 10px 12px;
  cursor: pointer;
  text-align: left;
  width: 100%;
  transition: border-color 140ms ease, background 140ms ease;
}

.subagent-card:hover {
  border-color: rgba(196, 178, 148, 0.9);
  background: rgba(245, 243, 238, 0.95);
}

.dark .subagent-card {
  border-color: #38342e;
  background: rgba(255, 255, 255, 0.03);
}

.dark .subagent-card:hover {
  border-color: #55503f;
  background: rgba(255, 255, 255, 0.06);
}

.subagent-card__head {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
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
  flex-shrink: 0;
}

.subagent-card__task {
  display: -webkit-box;
  margin-top: 6px;
  font-size: 12.5px;
  line-height: 1.5;
  color: #1a1a1a;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.dark .subagent-card__task {
  color: #d6cec2;
}

.subagent-card__report {
  display: block;
  margin-top: 8px;
  border-left: 3px solid rgba(34, 197, 94, 0.5);
  padding-left: 8px;
}

.subagent-card__report-label {
  display: block;
  font-size: 10px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #8b816f;
  margin-bottom: 3px;
}

.subagent-card__report-text {
  display: -webkit-box;
  font-size: 12px;
  line-height: 1.55;
  color: #1a1a1a;
  white-space: pre-wrap;
  word-break: break-word;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.dark .subagent-card__report-text {
  color: #d6cec2;
}

/* ── 详情页 ── */

.subagent-detail {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 14px 16px 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.subagent-detail__status-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.subagent-detail__phase {
  font-size: 13px;
  font-weight: 600;
  color: #4f473c;
}

.dark .subagent-detail__phase {
  color: #d6cec2;
}

.subagent-detail__started {
  font-size: 11px;
  color: #8b816f;
  font-variant-numeric: tabular-nums;
}

.subagent-detail__section {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

/* 日志区占满剩余高度并在内部滚动 */
.subagent-detail__section--log {
  flex: 1;
  min-height: 160px;
}

.subagent-detail__label {
  font-size: 10px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: #8b816f;
}

.subagent-detail__task,
.subagent-detail__report {
  font-size: 13px;
  line-height: 1.6;
  color: #1a1a1a;
  white-space: pre-wrap;
  word-break: break-word;
}

.dark .subagent-detail__task,
.dark .subagent-detail__report {
  color: #d6cec2;
}

.subagent-detail__error-text {
  font-size: 12.5px;
  line-height: 1.55;
  color: #be123c;
  word-break: break-all;
}

.subagent-detail__report {
  border-left: 3px solid rgba(34, 197, 94, 0.5);
  padding-left: 8px;
}

.subagent-detail__log {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  border-radius: 8px;
  background: rgba(31, 29, 26, 0.05);
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dark .subagent-detail__log {
  background: rgba(0, 0, 0, 0.28);
}

.subagent-detail__log::-webkit-scrollbar {
  width: 5px;
}

.subagent-detail__log::-webkit-scrollbar-track {
  background: transparent;
}

.subagent-detail__log::-webkit-scrollbar-thumb {
  background: rgba(139, 129, 111, 0.35);
  border-radius: 999px;
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
