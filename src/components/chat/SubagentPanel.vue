<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import {
  initSubagentEvents,
  panelOpen,
  selectedSubagentId,
  subagentsFor,
  type SubagentEntry,
} from '../../features/chat/services/subagents';

const props = defineProps<{
  conversationId?: string | null;
}>();

onMounted(() => {
  void initSubagentEvents();
  window.addEventListener('keydown', handleGlobalKeydown);
});

onUnmounted(() => {
  window.removeEventListener('keydown', handleGlobalKeydown);
});

const entries = computed<SubagentEntry[]>(() => subagentsFor(props.conversationId));
const runningCount = computed(() => entries.value.filter((e) => e.phase === 'running').length);

const selectedEntry = computed<SubagentEntry | null>(() => {
  if (selectedSubagentId.value && selectedSubagentId.value !== '__all__') {
    const found = entries.value.find((e) => e.subId === selectedSubagentId.value);
    if (found) return found;
  }
  // 若只有一个子代理且未明确看总览，默认直接展示其详情
  if (entries.value.length === 1 && selectedSubagentId.value !== '__all__') {
    return entries.value[0];
  }
  return null;
});

// 会话切换时重置
watch(
  () => props.conversationId,
  () => {
    selectedSubagentId.value = null;
  },
);

// 当面板首次打开且有子代理时，若未指定，默认选中首个子代理
watch(
  () => panelOpen.value,
  (open) => {
    if (open && entries.value.length > 0 && !selectedSubagentId.value) {
      selectedSubagentId.value = entries.value[0].subId;
    }
  },
);

const handleBackToMainAgent = () => {
  panelOpen.value = false;
};

const handleSelectSubagent = (subId: string) => {
  selectedSubagentId.value = subId;
  void nextTick(scrollLogToBottom);
};

const showAllOverview = () => {
  selectedSubagentId.value = '__all__';
};

const formatElapsed = (ms?: number) => {
  if (typeof ms !== 'number') return '';
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m${seconds % 60}s`;
};

const phaseLabel = (entry: SubagentEntry) =>
  entry.phase === 'running' ? '运行中' : entry.phase === 'error' ? '出现异常' : '已完成';

// 详情页日志容器：流式输出时自动滚到底部
const logEl = ref<HTMLElement | null>(null);
const autoScroll = ref(true);

const scrollLogToBottom = () => {
  const el = logEl.value;
  if (el) {
    el.scrollTop = el.scrollHeight;
  }
};

const handleLogScroll = () => {
  const el = logEl.value;
  if (!el) return;
  const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  autoScroll.value = isNearBottom;
};

watch(
  () => selectedEntry.value?.lines.length,
  () => {
    if (autoScroll.value) {
      void nextTick(scrollLogToBottom);
    }
  },
);

function handleGlobalKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && panelOpen.value) {
    e.preventDefault();
    handleBackToMainAgent();
  }
}

// 复制提示
const copiedTask = ref(false);
const copyTaskText = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
    copiedTask.value = true;
    setTimeout(() => {
      copiedTask.value = false;
    }, 1500);
  } catch {
    // ignore
  }
};

const copiedReport = ref(false);
const copyReportText = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text);
    copiedReport.value = true;
    setTimeout(() => {
      copiedReport.value = false;
    }, 1500);
  } catch {
    // ignore
  }
};
</script>

<template>
  <Teleport to="body">
    <!-- 右半屏（占 50% 视口）子代理面板 -->
    <Transition name="subagent-half-slide">
      <aside
        v-if="panelOpen && entries.length > 0"
        class="subagent-half-view"
        role="complementary"
        aria-label="子代理半屏详情面板"
      >
        <!-- 顶部导航栏 -->
        <header class="subagent-hf-header">
          <div class="subagent-hf-header__left">
            <!-- 核心：返回主代理按钮 -->
            <button
              type="button"
              class="subagent-back-btn"
              title="收起子代理，返回主代理 (Esc)"
              @click="handleBackToMainAgent"
            >
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="m15 18-6-6 6-6" />
              </svg>
              <span class="subagent-back-btn__label">返回主代理</span>
              <kbd class="subagent-back-btn__kbd">Esc</kbd>
            </button>

            <span class="subagent-hf-divider" aria-hidden="true" />

            <!-- 多子代理横向切换 Tabs -->
            <div v-if="entries.length > 1" class="subagent-tabs-group">
              <button
                type="button"
                class="subagent-tab"
                :class="{ 'subagent-tab--active': selectedSubagentId === '__all__' }"
                @click="showAllOverview"
              >
                <span class="subagent-tab__name">总览 ({{ entries.length }})</span>
              </button>

              <button
                v-for="(entry, index) in entries"
                :key="entry.subId"
                type="button"
                class="subagent-tab"
                :class="{ 'subagent-tab--active': selectedEntry?.subId === entry.subId }"
                @click="handleSelectSubagent(entry.subId)"
              >
                <span
                  class="subagent-tab__status-dot"
                  :class="{
                    'subagent-tab__status-dot--running': entry.phase === 'running',
                    'subagent-tab__status-dot--done': entry.phase === 'done',
                    'subagent-tab__status-dot--error': entry.phase === 'error',
                  }"
                />
                <span class="subagent-tab__name">子代理 #{{ index + 1 }}</span>
                <span v-if="formatElapsed(entry.elapsedMs)" class="subagent-tab__elapsed">
                  {{ formatElapsed(entry.elapsedMs) }}
                </span>
              </button>
            </div>

            <div v-else class="subagent-single-title">
              <span class="font-semibold text-[14px] text-[#0f172a] dark:text-[#f8fafc]">子代理详情</span>
            </div>
          </div>

          <div class="subagent-hf-header__right">
            <!-- 状态 Badge 与时间元数据 -->
            <template v-if="selectedEntry">
              <div
                class="subagent-status-badge"
                :class="{
                  'subagent-status-badge--running': selectedEntry.phase === 'running',
                  'subagent-status-badge--done': selectedEntry.phase === 'done',
                  'subagent-status-badge--error': selectedEntry.phase === 'error',
                }"
              >
                <span
                  class="subagent-status-badge__dot"
                  :class="{
                    'subagent-status-badge__dot--pulse': selectedEntry.phase === 'running',
                  }"
                />
                <span>{{ phaseLabel(selectedEntry) }}</span>
              </div>
            </template>

            <template v-else>
              <span v-if="runningCount > 0" class="subagent-status-badge subagent-status-badge--running">
                <span class="subagent-status-badge__dot subagent-status-badge__dot--pulse" />
                {{ runningCount }} 运行中
              </span>
            </template>

            <!-- 快捷关闭按钮 -->
            <button
              type="button"
              class="subagent-close-btn"
              title="收起子代理面板"
              @click="handleBackToMainAgent"
            >
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" aria-hidden="true">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>
        </header>

        <!-- 半屏主体滚动视图 -->
        <main class="subagent-hf-main custom-scrollbar">
          <!-- 模式 1：单条子代理详情 -->
          <div v-if="selectedEntry" class="subagent-detail-container">
            <!-- 1. 任务描述卡片 -->
            <section class="subagent-card subagent-card--mission">
              <div class="subagent-card__header">
                <div class="flex items-center gap-2">
                  <span class="subagent-card__icon text-[#2563eb] dark:text-[#60a5fa]">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
                      <rect x="8" y="2" width="8" height="4" rx="1" ry="1" />
                    </svg>
                  </span>
                  <h2 class="subagent-card__title">调查任务 / 运行提示词</h2>
                </div>
                <div class="flex items-center gap-2">
                  <span v-if="formatElapsed(selectedEntry.elapsedMs)" class="text-xs text-[#64748b] dark:text-[#94a3b8] font-mono">
                    耗时 {{ formatElapsed(selectedEntry.elapsedMs) }}
                  </span>
                  <button
                    type="button"
                    class="subagent-action-btn"
                    :title="copiedTask ? '已复制' : '复制任务指令'"
                    @click="copyTaskText(selectedEntry.task)"
                  >
                    <svg v-if="!copiedTask" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                    </svg>
                    <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                    <span>{{ copiedTask ? '已复制' : '复制' }}</span>
                  </button>
                </div>
              </div>

              <div class="subagent-card__body">
                <div class="subagent-mission-content">
                  {{ selectedEntry.task }}
                </div>
              </div>
            </section>

            <!-- 2. 异常信息卡片（若出错） -->
            <section v-if="selectedEntry.detail" class="subagent-card subagent-card--error">
              <div class="subagent-card__header text-[#dc2626] dark:text-[#f87171]">
                <div class="flex items-center gap-2">
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <circle cx="12" cy="12" r="10" />
                    <line x1="12" y1="8" x2="12" y2="12" />
                    <line x1="12" y1="16" x2="12.01" y2="16" />
                  </svg>
                  <h2 class="subagent-card__title">异常中断原因</h2>
                </div>
              </div>
              <div class="subagent-card__body">
                <div class="text-[13.5px] text-[#dc2626] dark:text-[#f87171] leading-relaxed font-mono">
                  {{ selectedEntry.detail }}
                </div>
              </div>
            </section>

            <!-- 3. 交付报告卡片（若有） -->
            <section v-if="selectedEntry.reportPreview" class="subagent-card subagent-card--report">
              <div class="subagent-card__header text-[#16a34a] dark:text-[#4ade80]">
                <div class="flex items-center gap-2">
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                    <polyline points="14 2 14 8 20 8" />
                    <line x1="16" y1="13" x2="8" y2="13" />
                    <line x1="16" y1="17" x2="8" y2="17" />
                    <polyline points="10 9 9 9 8 9" />
                  </svg>
                  <h2 class="subagent-card__title">交付成果报告</h2>
                </div>
                <button
                  type="button"
                  class="subagent-action-btn"
                  @click="copyReportText(selectedEntry.reportPreview)"
                >
                  <span>{{ copiedReport ? '已复制' : '复制报告' }}</span>
                </button>
              </div>
              <div class="subagent-card__body">
                <div class="subagent-report-content">
                  {{ selectedEntry.reportPreview }}
                </div>
              </div>
            </section>

            <!-- 4. 实时运行日志与执行步骤 -->
            <section class="subagent-card subagent-card--terminal">
              <div class="subagent-card__header subagent-card__header--terminal">
                <div class="flex items-center gap-2">
                  <span class="subagent-terminal-dot subagent-terminal-dot--red" />
                  <span class="subagent-terminal-dot subagent-terminal-dot--yellow" />
                  <span class="subagent-terminal-dot subagent-terminal-dot--green" />
                  <h2 class="subagent-card__title font-mono text-[12.5px] ml-1">执行轨迹 ({{ selectedEntry.lines.length }} 条记录)</h2>
                </div>

                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    class="subagent-action-btn subagent-action-btn--sm"
                    :class="{ 'opacity-100 font-semibold': autoScroll, 'opacity-60': !autoScroll }"
                    @click="autoScroll = !autoScroll; scrollLogToBottom()"
                  >
                    <span>{{ autoScroll ? '滚动锁定' : '自由浏览' }}</span>
                  </button>
                </div>
              </div>

              <div
                ref="logEl"
                class="subagent-terminal-window custom-scrollbar"
                @scroll="handleLogScroll"
              >
                <div v-if="selectedEntry.lines.length === 0" class="subagent-log-empty">
                  <div class="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-solid border-current border-r-transparent mr-2" />
                  等待子代理执行流输出…
                </div>

                <div class="subagent-log-lines">
                  <template v-for="(line, index) in selectedEntry.lines" :key="index">
                    <!-- 工具调用发起 -->
                    <div v-if="line.kind === 'tool'" class="subagent-log-item subagent-log-item--tool">
                      <span class="subagent-log-tool-tag">TOOL</span>
                      <span class="subagent-log-tool-name">{{ line.toolName }}</span>
                      <span class="text-[11px] text-[#94a3b8] dark:text-[#64748b]">发起调用</span>
                    </div>

                    <!-- 工具执行结果 -->
                    <div
                      v-else-if="line.kind === 'tool-result'"
                      class="subagent-log-item subagent-log-item--result"
                      :class="{ 'subagent-log-item--result-error': line.isError }"
                    >
                      <div class="flex items-center gap-1.5 text-[11.5px] font-semibold text-[#64748b] dark:text-[#94a3b8] mb-1">
                        <span>↳ {{ line.toolName || '工具返回' }}</span>
                        <span v-if="line.isError" class="text-red-500 font-normal">[执行异常]</span>
                      </div>
                      <pre class="subagent-log-pre">{{ line.text }}</pre>
                    </div>

                    <!-- 思维链推理 Reasoning -->
                    <div v-else-if="line.kind === 'reasoning'" class="subagent-log-item subagent-log-item--reasoning">
                      <div class="text-[10.5px] font-semibold tracking-wider uppercase text-[#8b5cf6] dark:text-[#a78bfa] mb-1">
                        Thinking · 思考
                      </div>
                      <div class="subagent-log-reasoning-body">{{ line.text }}</div>
                    </div>

                    <!-- 普通模型正文输出 -->
                    <div v-else class="subagent-log-item subagent-log-item--text">
                      {{ line.text }}
                    </div>
                  </template>
                </div>
              </div>
            </section>
          </div>

          <!-- 模式 2：全部子代理概览大卡片 -->
          <div v-else class="subagent-overview-container">
            <div class="mb-4">
              <h2 class="text-lg font-bold text-[#0f172a] dark:text-[#f8fafc]">所有子代理任务 ({{ entries.length }})</h2>
              <p class="text-xs text-[#64748b] dark:text-[#94a3b8] mt-0.5">选择任意卡片查看详细执行轨迹与日志</p>
            </div>

            <div class="subagent-grid">
              <div
                v-for="(entry, index) in entries"
                :key="entry.subId"
                class="subagent-overview-card"
                role="button"
                tabindex="0"
                @click="handleSelectSubagent(entry.subId)"
                @keydown.enter="handleSelectSubagent(entry.subId)"
              >
                <div class="subagent-overview-card__top">
                  <div class="flex items-center gap-2">
                    <span class="font-bold text-[14px] text-[#0f172a] dark:text-[#f8fafc]">
                      子代理 #{{ index + 1 }}
                    </span>
                    <span
                      class="subagent-status-badge subagent-status-badge--sm"
                      :class="{
                        'subagent-status-badge--running': entry.phase === 'running',
                        'subagent-status-badge--done': entry.phase === 'done',
                        'subagent-status-badge--error': entry.phase === 'error',
                      }"
                    >
                      <span
                        class="subagent-status-badge__dot"
                        :class="{ 'subagent-status-badge__dot--pulse': entry.phase === 'running' }"
                      />
                      {{ phaseLabel(entry) }}
                    </span>
                  </div>

                  <span v-if="formatElapsed(entry.elapsedMs)" class="text-xs font-medium text-[#64748b] dark:text-[#94a3b8]">
                    耗时 {{ formatElapsed(entry.elapsedMs) }}
                  </span>
                </div>

                <div class="subagent-overview-card__task line-clamp-3">
                  {{ entry.task }}
                </div>

                <div v-if="entry.reportPreview" class="subagent-overview-card__report line-clamp-2">
                  <span class="font-medium text-[#16a34a] dark:text-[#4ade80]">报告：</span>
                  {{ entry.reportPreview }}
                </div>

                <div class="subagent-overview-card__footer">
                  <span class="text-xs text-[#94a3b8] dark:text-[#64748b]">
                    {{ entry.lines.length }} 条运行输出
                  </span>
                  <span class="subagent-overview-card__enter-btn">
                    进入详情
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
                      <path d="M5 12h14" />
                      <path d="m12 5 7 7-7 7" />
                    </svg>
                  </span>
                </div>
              </div>
            </div>
          </div>
        </main>
      </aside>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* ── 右半屏面板容器（占 50% 视口，高度 100%） ── */
.subagent-half-view {
  position: fixed;
  top: 0;
  bottom: 0;
  right: 0;
  width: 50vw;
  min-width: 480px;
  max-width: 65vw;
  z-index: 60;
  display: flex;
  flex-direction: column;
  background-color: #f8fafc;
  color: #0f172a;
  border-left: 1px solid #e2e8f0;
  box-shadow: -8px 0 32px -4px rgba(0, 0, 0, 0.08), -2px 0 8px -2px rgba(0, 0, 0, 0.04);
  overflow: hidden;
  font-family: inherit;
}

:global(.dark) .subagent-half-view {
  background-color: #0c0d10;
  color: #f8fafc;
  border-left-color: #1e2025;
  box-shadow: -12px 0 36px -4px rgba(0, 0, 0, 0.5);
}

/* ── 顶部半屏导航栏 ── */
.subagent-hf-header {
  height: 56px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  border-bottom: 1px solid #e2e8f0;
  background-color: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(16px);
  flex-shrink: 0;
}

:global(.dark) .subagent-hf-header {
  border-bottom-color: #1a1c21;
  background-color: rgba(16, 17, 21, 0.95);
}

.subagent-hf-header__left {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}

.subagent-hf-header__right {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

/* 核心：返回主代理按钮 */
.subagent-back-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 11px;
  border-radius: 8px;
  background: #ffffff;
  border: 1px solid #cbd5e1;
  color: #1e293b;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
  transition: all 150ms cubic-bezier(0.16, 1, 0.3, 1);
}

.subagent-back-btn:hover {
  background: #f1f5f9;
  border-color: #94a3b8;
  color: #0f172a;
  transform: translateY(-0.5px);
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.06);
}

.subagent-back-btn:active {
  transform: translateY(0.5px);
}

:global(.dark) .subagent-back-btn {
  background: #1a1b1f;
  border-color: #2e3138;
  color: #e2e8f0;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
}

:global(.dark) .subagent-back-btn:hover {
  background: #25272e;
  border-color: #424650;
  color: #ffffff;
}

.subagent-back-btn__label {
  letter-spacing: -0.01em;
}

.subagent-back-btn__kbd {
  font-size: 10px;
  font-family: inherit;
  padding: 1px 4px;
  border-radius: 4px;
  background: #f1f5f9;
  border: 1px solid #cbd5e1;
  color: #64748b;
  font-weight: 500;
}

:global(.dark) .subagent-back-btn__kbd {
  background: #25272d;
  border-color: #353842;
  color: #94a3b8;
}

.subagent-hf-divider {
  width: 1px;
  height: 20px;
  background-color: #cbd5e1;
}

:global(.dark) .subagent-hf-divider {
  background-color: #272a30;
}

/* 顶部 Tab 组 */
.subagent-tabs-group {
  display: flex;
  align-items: center;
  gap: 5px;
  overflow-x: auto;
  padding: 2px 0;
}

.subagent-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 9px;
  border-radius: 7px;
  font-size: 12.5px;
  font-weight: 500;
  border: 1px solid transparent;
  background: transparent;
  color: #64748b;
  cursor: pointer;
  transition: all 120ms ease;
  white-space: nowrap;
}

.subagent-tab:hover {
  background: rgba(0, 0, 0, 0.04);
  color: #1e293b;
}

:global(.dark) .subagent-tab {
  color: #94a3b8;
}

:global(.dark) .subagent-tab:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #f8fafc;
}

.subagent-tab--active {
  background: #ffffff !important;
  color: #0f172a !important;
  border-color: #e2e8f0;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
}

:global(.dark) .subagent-tab--active {
  background: #1a1b1f !important;
  color: #ffffff !important;
  border-color: #2a2d34;
}

.subagent-tab__status-dot {
  width: 6.5px;
  height: 6.5px;
  border-radius: 9999px;
  background: #94a3b8;
}

.subagent-tab__status-dot--running {
  background: #22c55e;
  box-shadow: 0 0 6px rgba(34, 197, 94, 0.6);
}

.subagent-tab__status-dot--done { background: #3b82f6; }
.subagent-tab__status-dot--error { background: #ef4444; }

.subagent-tab__elapsed {
  font-size: 11px;
  color: #94a3b8;
  font-variant-numeric: tabular-nums;
}

/* 状态徽章 */
.subagent-status-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  border-radius: 9999px;
  font-size: 11.5px;
  font-weight: 600;
}

.subagent-status-badge--sm {
  padding: 2px 6px;
  font-size: 11px;
}

.subagent-status-badge--running {
  background: #dcfce7;
  color: #15803d;
  border: 1px solid #bbf7d0;
}

:global(.dark) .subagent-status-badge--running {
  background: rgba(20, 83, 45, 0.4);
  color: #86efac;
  border-color: #166534;
}

.subagent-status-badge--done {
  background: #f1f5f9;
  color: #475569;
  border: 1px solid #e2e8f0;
}

:global(.dark) .subagent-status-badge--done {
  background: #18191d;
  color: #94a3b8;
  border-color: #272a31;
}

.subagent-status-badge--error {
  background: #fee2e2;
  color: #b91c1c;
  border: 1px solid #fecaca;
}

:global(.dark) .subagent-status-badge--error {
  background: rgba(127, 29, 29, 0.4);
  color: #fca5a5;
  border-color: #991b1b;
}

.subagent-status-badge__dot {
  width: 6px;
  height: 6px;
  border-radius: 9999px;
  background: currentColor;
}

.subagent-status-badge__dot--pulse {
  animation: subagent-pulse-glow 1.5s infinite ease-in-out;
}

@keyframes subagent-pulse-glow {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.4; transform: scale(0.85); }
}

.subagent-close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 7px;
  border: 0;
  background: transparent;
  color: #64748b;
  cursor: pointer;
  transition: all 120ms ease;
}

.subagent-close-btn:hover {
  background: #e2e8f0;
  color: #0f172a;
}

:global(.dark) .subagent-close-btn:hover {
  background: #202228;
  color: #ffffff;
}

/* ── 半屏主体滚动区 ── */
.subagent-hf-main {
  flex: 1;
  overflow-y: auto;
  padding: 18px 18px 40px;
}

.subagent-detail-container {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* ── 卡片通用样式 ── */
.subagent-card {
  border-radius: 14px;
  border: 1px solid #e2e8f0;
  background-color: #ffffff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.02);
  overflow: hidden;
  transition: border-color 150ms ease;
}

:global(.dark) .subagent-card {
  border-color: #1c1d22;
  background-color: #121316;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
}

.subagent-card__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-bottom: 1px solid #f1f5f9;
}

:global(.dark) .subagent-card__header {
  border-bottom-color: #18191d;
}

.subagent-card__title {
  font-size: 13.5px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.subagent-card__body {
  padding: 14px 16px;
}

/* 任务内容 */
.subagent-mission-content {
  font-size: 13.5px;
  line-height: 1.65;
  color: #1e293b;
  white-space: pre-wrap;
  word-break: break-word;
}

:global(.dark) .subagent-mission-content {
  color: #e2e8f0;
}

/* 交付报告卡片 */
.subagent-card--report {
  border-color: #bbf7d0;
  background-color: #f0fdf4;
}

:global(.dark) .subagent-card--report {
  border-color: #14532d;
  background-color: rgba(5, 46, 22, 0.25);
}

.subagent-report-content {
  font-size: 13.5px;
  line-height: 1.65;
  color: #14532d;
  white-space: pre-wrap;
  word-break: break-word;
}

:global(.dark) .subagent-report-content {
  color: #bbf7d0;
}

/* 异常卡片 */
.subagent-card--error {
  border-color: #fecaca;
  background-color: #fef2f2;
}

:global(.dark) .subagent-card--error {
  border-color: #7f1d1d;
  background-color: rgba(69, 10, 10, 0.3);
}

/* 操作小按钮 */
.subagent-action-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 6px;
  font-size: 11.5px;
  font-weight: 500;
  border: 1px solid #cbd5e1;
  background: #ffffff;
  color: #475569;
  cursor: pointer;
  transition: all 120ms ease;
}

.subagent-action-btn:hover {
  background: #f1f5f9;
  color: #0f172a;
}

:global(.dark) .subagent-action-btn {
  border-color: #2e3138;
  background: #1a1b1f;
  color: #94a3b8;
}

:global(.dark) .subagent-action-btn:hover {
  background: #25272e;
  color: #f8fafc;
}

.subagent-action-btn--sm {
  padding: 2px 6px;
  font-size: 11px;
}

/* 终端输出卡片 */
.subagent-card--terminal {
  border-color: #cbd5e1;
  background-color: #ffffff;
}

:global(.dark) .subagent-card--terminal {
  border-color: #23252c;
  background-color: #0e0f12;
}

.subagent-card__header--terminal {
  background: #f8fafc;
  padding: 9px 15px;
}

:global(.dark) .subagent-card__header--terminal {
  background: #141519;
}

.subagent-terminal-dot {
  width: 8.5px;
  height: 8.5px;
  border-radius: 9999px;
}

.subagent-terminal-dot--red { background: #ef4444; }
.subagent-terminal-dot--yellow { background: #f59e0b; }
.subagent-terminal-dot--green { background: #10b981; }

.subagent-terminal-window {
  min-height: 240px;
  max-height: 520px;
  overflow-y: auto;
  padding: 14px 16px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  font-size: 12.5px;
  line-height: 1.6;
  background: #ffffff;
}

:global(.dark) .subagent-terminal-window {
  background: #0e0f12;
}

.subagent-log-empty {
  padding: 36px 0;
  text-align: center;
  color: #94a3b8;
  font-size: 12.5px;
}

.subagent-log-lines {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.subagent-log-item {
  word-break: break-word;
}

/* 工具卡片项 */
.subagent-log-item--tool {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 9px;
  border-radius: 7px;
  background: #f1f5f9;
  border: 1px solid #e2e8f0;
}

:global(.dark) .subagent-log-item--tool {
  background: #17181d;
  border-color: #24262d;
}

.subagent-log-tool-tag {
  font-size: 9.5px;
  font-weight: 700;
  padding: 1px 4px;
  border-radius: 4px;
  background: #3b82f6;
  color: #ffffff;
}

.subagent-log-tool-name {
  font-weight: 600;
  color: #0f172a;
}

:global(.dark) .subagent-log-tool-name {
  color: #f8fafc;
}

/* 结果项 */
.subagent-log-item--result {
  padding: 7px 10px;
  border-radius: 7px;
  background: #f8fafc;
  border: 1px solid #e2e8f0;
  margin-left: 10px;
}

:global(.dark) .subagent-log-item--result {
  background: #121417;
  border-color: #1e2025;
}

.subagent-log-item--result-error {
  border-color: #fecaca;
  background: #fef2f2;
}

:global(.dark) .subagent-log-item--result-error {
  border-color: #7f1d1d;
  background: rgba(69, 10, 10, 0.25);
}

.subagent-log-pre {
  margin: 0;
  font-family: inherit;
  font-size: 11.5px;
  line-height: 1.5;
  white-space: pre-wrap;
  color: #334155;
}

:global(.dark) .subagent-log-pre {
  color: #cbd5e1;
}

/* 思考链推理 */
.subagent-log-item--reasoning {
  padding: 8px 12px;
  border-radius: 9px;
  background: #faf5ff;
  border: 1px solid #f3e8ff;
  border-left: 3px solid #8b5cf6;
}

:global(.dark) .subagent-log-item--reasoning {
  background: rgba(88, 28, 135, 0.12);
  border-color: rgba(139, 92, 246, 0.2);
  border-left-color: #a78bfa;
}

.subagent-log-reasoning-body {
  font-size: 12px;
  color: #4c1d95;
  white-space: pre-wrap;
}

:global(.dark) .subagent-log-reasoning-body {
  color: #d8b4fe;
}

/* 普通正文 */
.subagent-log-item--text {
  font-family: inherit;
  font-size: 13px;
  line-height: 1.6;
  white-space: pre-wrap;
  color: #1e293b;
}

:global(.dark) .subagent-log-item--text {
  color: #e2e8f0;
}

/* ── 模式 2：总览网格 ── */
.subagent-overview-container {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.subagent-grid {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.subagent-overview-card {
  display: flex;
  flex-direction: column;
  padding: 16px;
  border-radius: 12px;
  border: 1px solid #e2e8f0;
  background: #ffffff;
  cursor: pointer;
  transition: all 140ms ease;
}

.subagent-overview-card:hover {
  border-color: #94a3b8;
  transform: translateY(-1.5px);
  box-shadow: 0 6px 16px -3px rgba(0, 0, 0, 0.05);
}

:global(.dark) .subagent-overview-card {
  border-color: #1c1e23;
  background: #121316;
}

:global(.dark) .subagent-overview-card:hover {
  border-color: #353842;
  box-shadow: 0 6px 20px -3px rgba(0, 0, 0, 0.4);
}

.subagent-overview-card__top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}

.subagent-overview-card__task {
  font-size: 13px;
  line-height: 1.5;
  color: #334155;
  margin-bottom: 10px;
}

:global(.dark) .subagent-overview-card__task {
  color: #cbd5e1;
}

.subagent-overview-card__report {
  font-size: 12px;
  line-height: 1.45;
  padding: 6px 9px;
  border-radius: 7px;
  background: #f0fdf4;
  border: 1px solid #dcfce7;
  color: #166534;
  margin-bottom: 10px;
}

:global(.dark) .subagent-overview-card__report {
  background: rgba(5, 46, 22, 0.3);
  border-color: #14532d;
  color: #86efac;
}

.subagent-overview-card__footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-top: 8px;
  border-top: 1px solid #f1f5f9;
}

:global(.dark) .subagent-overview-card__footer {
  border-top-color: #18191e;
}

.subagent-overview-card__enter-btn {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 12px;
  font-weight: 600;
  color: #2563eb;
}

:global(.dark) .subagent-overview-card__enter-btn {
  color: #60a5fa;
}

/* ── 半屏侧边滑入/滑出动画 ── */
.subagent-half-slide-enter-active,
.subagent-half-slide-leave-active {
  transition: transform 220ms cubic-bezier(0.16, 1, 0.3, 1), opacity 180ms ease;
}

.subagent-half-slide-enter-from,
.subagent-half-slide-leave-to {
  transform: translateX(100%);
  opacity: 0;
}
</style>
