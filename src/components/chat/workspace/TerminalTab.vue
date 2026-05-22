<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { ToolExecutionEntry } from '../../../lib/chat-types';
import {
  getShellSessionStatus,
  resetShellSessionForConversation,
  type ShellSessionStatus,
} from '../../../features/chat/services/chat-api';
import { emitToast } from '../../../lib/toast';

const props = defineProps<{
  conversationId: string | null;
  entries: ToolExecutionEntry[];
  currentTurnToolEntries?: ToolExecutionEntry[];
}>();

const isLoading = ref(false);
const isResetting = ref(false);
const status = ref<ShellSessionStatus | null>(null);
let refreshTimer: ReturnType<typeof setInterval> | undefined;

const isShellToolName = (toolName: string) => {
  const normalized = toolName.trim().toLowerCase();
  return (
    normalized === 'execute_bash' ||
    normalized === 'execute_powershell' ||
    normalized === 'reset_shell_session'
  );
};

const shellEntries = computed(() =>
  props.entries
    .filter((entry) => isShellToolName(entry.toolName))
    .slice(-10)
    .reverse(),
);

const hasRunningShellTool = computed(() =>
  (props.currentTurnToolEntries ?? props.entries).some(
    (entry) => entry.status === 'running' && isShellToolName(entry.toolName),
  ),
);

const statusLabel = computed(() => {
  if (hasRunningShellTool.value) return '执行中';
  if (!status.value?.exists) return '未启动';
  if (status.value.busy) return '执行中';
  if (!status.value.alive) return '已断开';
  if (status.value.backgroundCount > 0) return `后台 ${status.value.backgroundCount}`;
  return '空闲';
});

const statusClass = computed(() => {
  if (hasRunningShellTool.value || status.value?.busy) {
    return 'terminal-status-busy';
  }
  if (!status.value?.exists) {
    return 'terminal-status-idle';
  }
  if (!status.value.alive) {
    return 'terminal-status-error';
  }
  if (status.value.backgroundCount > 0) {
    return 'terminal-status-background';
  }
  return 'terminal-status-ready';
});

const currentCwd = computed(() => status.value?.cwd || '尚未创建终端会话');
const backgroundPids = computed(() => status.value?.backgroundPids ?? []);

const loadStatus = async () => {
  if (!props.conversationId) {
    status.value = null;
    return;
  }

  isLoading.value = true;
  try {
    const next = await getShellSessionStatus(props.conversationId);
    status.value = next.busy
      ? {
          ...next,
          cwd: next.cwd ?? status.value?.cwd ?? null,
          backgroundPids:
            next.backgroundPids.length > 0
              ? next.backgroundPids
              : status.value?.backgroundPids ?? [],
          backgroundCount:
            next.backgroundCount > 0 ? next.backgroundCount : status.value?.backgroundCount ?? 0,
        }
      : next;
  } catch (error) {
    console.error('Failed to load shell session status:', error);
    emitToast({ variant: 'error', source: 'terminal-tab', message: '读取终端状态失败。' });
  } finally {
    isLoading.value = false;
  }
};

const resetSession = async () => {
  if (!props.conversationId || isResetting.value) return;
  isResetting.value = true;
  try {
    await resetShellSessionForConversation(props.conversationId);
    await loadStatus();
    emitToast({ variant: 'success', source: 'terminal-tab', message: '终端会话已重置。' });
  } catch (error) {
    console.error('Failed to reset shell session:', error);
    emitToast({ variant: 'error', source: 'terminal-tab', message: '重置终端会话失败。' });
  } finally {
    isResetting.value = false;
  }
};

const formatTime = (timestamp: number) =>
  new Date(timestamp).toLocaleTimeString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });

const formatDuration = (entry: ToolExecutionEntry) => {
  if (!entry.finishedAt) return '运行中';
  const duration = Math.max(0, entry.finishedAt - entry.startedAt);
  return duration >= 1000 ? `${(duration / 1000).toFixed(1)}s` : `${duration}ms`;
};

const entryStatusLabel = (statusValue: ToolExecutionEntry['status']) => {
  if (statusValue === 'completed') return '完成';
  if (statusValue === 'error') return '失败';
  if (statusValue === 'cancelled') return '取消';
  return '运行中';
};

watch(
  () => props.conversationId,
  () => {
    status.value = null;
    void loadStatus();
  },
  { immediate: true },
);

watch(
  () => [props.entries.length, hasRunningShellTool.value],
  () => {
    void loadStatus();
  },
);

onMounted(() => {
  refreshTimer = setInterval(() => {
    if (status.value?.busy || hasRunningShellTool.value) {
      void loadStatus();
    }
  }, 1500);
});

onBeforeUnmount(() => {
  if (refreshTimer) {
    clearInterval(refreshTimer);
  }
});
</script>

<template>
  <div class="terminal-tab h-full overflow-y-auto bg-[#faf9f6] px-4 py-4 dark:bg-[#1e1e1e]">
    <div class="terminal-hero">
      <div>
        <div class="terminal-kicker">Shell Session</div>
        <h2 class="terminal-title">终端</h2>
        <p class="terminal-subtitle">
          这里是会话级终端页壳，展示 agent 的 execute_bash / execute_powershell 持久会话状态。
        </p>
      </div>
      <div class="terminal-actions">
        <span class="terminal-status-pill" :class="statusClass">{{ isLoading && !status ? '读取中' : statusLabel }}</span>
        <button type="button" class="terminal-action-button" :disabled="isLoading" @click="loadStatus">
          {{ isLoading ? '刷新中' : '刷新' }}
        </button>
        <button type="button" class="terminal-action-button terminal-action-danger" :disabled="!conversationId || isResetting" @click="resetSession">
          {{ isResetting ? '重置中' : '重置会话' }}
        </button>
      </div>
    </div>

    <div class="mt-4 grid grid-cols-2 gap-3">
      <div class="terminal-card">
        <div class="terminal-label">状态</div>
        <div class="terminal-value">{{ statusLabel }}</div>
      </div>
      <div class="terminal-card">
        <div class="terminal-label">后台进程</div>
        <div class="terminal-value">{{ status?.backgroundCount ?? 0 }}</div>
      </div>
      <div class="terminal-card col-span-2">
        <div class="terminal-label">当前目录</div>
        <code class="terminal-code">{{ currentCwd }}</code>
      </div>
      <div class="terminal-card col-span-2">
        <div class="terminal-label">会话</div>
        <code class="terminal-code">{{ conversationId || '当前还没有选中的会话' }}</code>
      </div>
    </div>

    <div v-if="backgroundPids.length" class="terminal-panel">
      <div class="terminal-panel-title">后台 PID</div>
      <div class="terminal-pid-list">
        <span v-for="pid in backgroundPids" :key="pid">{{ pid }}</span>
      </div>
    </div>

    <div class="terminal-panel">
      <div class="terminal-panel-header">
        <div>
          <div class="terminal-panel-title">最近终端工具</div>
          <div class="terminal-panel-subtitle">展示最近 10 条 shell 工具记录。</div>
        </div>
        <span class="terminal-count">{{ shellEntries.length }}</span>
      </div>

      <div v-if="shellEntries.length === 0" class="terminal-empty">
        还没有终端工具调用。agent 第一次执行 shell 命令后，这里会显示状态和摘要。
      </div>

      <div v-else class="terminal-entry-list">
        <article v-for="entry in shellEntries" :key="entry.id" class="terminal-entry">
          <div class="terminal-entry-top">
            <div class="min-w-0">
              <div class="terminal-entry-name">{{ entry.toolName }}</div>
              <div class="terminal-entry-meta">
                {{ formatTime(entry.startedAt) }} · {{ formatDuration(entry) }}
              </div>
            </div>
            <span class="terminal-entry-status" :class="`terminal-entry-status-${entry.status}`">
              {{ entryStatusLabel(entry.status) }}
            </span>
          </div>
          <pre v-if="entry.input" class="terminal-entry-snippet">{{ entry.input }}</pre>
          <pre v-if="entry.result" class="terminal-entry-snippet terminal-entry-result">{{ entry.result }}</pre>
        </article>
      </div>
    </div>
  </div>
</template>

<style scoped>
.terminal-hero {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  border: 1px solid #e7e2d7;
  border-radius: 22px;
  background:
    radial-gradient(circle at 0% 0%, rgba(218, 119, 86, 0.15), transparent 34%),
    linear-gradient(135deg, rgba(255, 255, 255, 0.96), rgba(249, 245, 237, 0.86));
  padding: 18px;
}

.dark .terminal-hero {
  border-color: #333;
  background:
    radial-gradient(circle at 0% 0%, rgba(218, 119, 86, 0.12), transparent 34%),
    linear-gradient(135deg, rgba(42, 42, 42, 0.96), rgba(31, 31, 31, 0.9));
}

.terminal-kicker,
.terminal-label,
.terminal-panel-subtitle,
.terminal-entry-meta {
  color: #8b8172;
}

.terminal-kicker,
.terminal-label {
  font-size: 11px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.terminal-title {
  margin-top: 4px;
  color: #201b14;
  font-size: 24px;
  font-weight: 720;
  letter-spacing: -0.03em;
}

.terminal-subtitle {
  margin-top: 6px;
  max-width: 520px;
  color: #746a5c;
  font-size: 13px;
  line-height: 1.7;
}

.dark .terminal-title {
  color: #f2eee8;
}

.dark .terminal-subtitle,
.dark .terminal-kicker,
.dark .terminal-label,
.dark .terminal-panel-subtitle,
.dark .terminal-entry-meta {
  color: #aaa197;
}

.terminal-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
}

.terminal-status-pill,
.terminal-action-button,
.terminal-count,
.terminal-entry-status,
.terminal-pid-list span {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 999px;
  font-size: 12px;
  line-height: 1;
}

.terminal-status-pill {
  min-height: 30px;
  padding: 0 11px;
  font-weight: 600;
}

.terminal-status-ready {
  background: #e4f4ed;
  color: #2f7553;
}

.terminal-status-background,
.terminal-status-busy {
  background: #fff3cf;
  color: #8a5a05;
}

.terminal-status-error {
  background: #fff0ea;
  color: #a6533c;
}

.terminal-status-idle {
  background: #efe9dc;
  color: #6f6657;
}

.dark .terminal-status-ready {
  background: #173428;
  color: #90d8b4;
}

.dark .terminal-status-background,
.dark .terminal-status-busy {
  background: #3c3016;
  color: #ffd37a;
}

.dark .terminal-status-error {
  background: #3a211b;
  color: #f0a08b;
}

.dark .terminal-status-idle {
  background: #3a342d;
  color: #d8d0c2;
}

.terminal-action-button {
  min-height: 30px;
  border: 1px solid #dfd5c5;
  background: rgba(255, 255, 255, 0.78);
  color: #665c4d;
  padding: 0 11px;
  transition:
    background 160ms ease,
    color 160ms ease,
    border-color 160ms ease;
}

.terminal-action-button:hover:not(:disabled) {
  border-color: #d2c4b0;
  background: #fff;
  color: #2d2922;
}

.terminal-action-button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.terminal-action-danger {
  color: #8a4b39;
}

.dark .terminal-action-button {
  border-color: #46413a;
  background: rgba(255, 255, 255, 0.05);
  color: #d5cdc0;
}

.dark .terminal-action-button:hover:not(:disabled) {
  border-color: #5c554b;
  background: rgba(255, 255, 255, 0.08);
  color: #f2eee8;
}

.terminal-card,
.terminal-panel {
  border: 1px solid #e7e2d7;
  border-radius: 18px;
  background: rgba(255, 255, 255, 0.78);
  padding: 14px;
}

.dark .terminal-card,
.dark .terminal-panel {
  border-color: #333;
  background: #252525;
}

.terminal-value {
  margin-top: 8px;
  color: #201b14;
  font-size: 22px;
  font-weight: 680;
}

.dark .terminal-value {
  color: #f2eee8;
}

.terminal-code {
  margin-top: 8px;
  display: block;
  overflow-wrap: anywhere;
  color: #2f2a22;
  font-size: 12px;
  line-height: 1.6;
}

.dark .terminal-code {
  color: #eee6dc;
}

.terminal-panel {
  margin-top: 14px;
}

.terminal-panel-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.terminal-panel-title {
  color: #201b14;
  font-size: 15px;
  font-weight: 680;
}

.terminal-panel-subtitle {
  margin-top: 4px;
  font-size: 12px;
}

.dark .terminal-panel-title {
  color: #f2eee8;
}

.terminal-count {
  min-width: 28px;
  height: 24px;
  background: #efe9dc;
  color: #6f6657;
  font-weight: 600;
}

.dark .terminal-count {
  background: #3a342d;
  color: #d8d0c2;
}

.terminal-pid-list {
  margin-top: 10px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.terminal-pid-list span {
  background: #f4eee2;
  color: #6f6657;
  padding: 7px 10px;
}

.dark .terminal-pid-list span {
  background: #302c27;
  color: #d8d0c2;
}

.terminal-empty {
  margin-top: 14px;
  border: 1px dashed #d8d0c2;
  border-radius: 14px;
  color: #8b8172;
  font-size: 13px;
  line-height: 1.7;
  padding: 22px;
  text-align: center;
}

.dark .terminal-empty {
  border-color: #444;
  color: #aaa197;
}

.terminal-entry-list {
  margin-top: 12px;
  display: grid;
  gap: 10px;
}

.terminal-entry {
  border: 1px solid #eee6da;
  border-radius: 15px;
  background: #fffdf8;
  padding: 12px;
}

.dark .terminal-entry {
  border-color: #393530;
  background: #292929;
}

.terminal-entry-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.terminal-entry-name {
  color: #201b14;
  font-size: 13px;
  font-weight: 650;
}

.dark .terminal-entry-name {
  color: #f2eee8;
}

.terminal-entry-meta {
  margin-top: 4px;
  font-size: 11px;
}

.terminal-entry-status {
  height: 24px;
  flex-shrink: 0;
  padding: 0 9px;
  font-weight: 600;
}

.terminal-entry-status-running {
  background: #fff3cf;
  color: #8a5a05;
}

.terminal-entry-status-completed {
  background: #e4f4ed;
  color: #2f7553;
}

.terminal-entry-status-error,
.terminal-entry-status-cancelled {
  background: #fff0ea;
  color: #a6533c;
}

.dark .terminal-entry-status-running {
  background: #3c3016;
  color: #ffd37a;
}

.dark .terminal-entry-status-completed {
  background: #173428;
  color: #90d8b4;
}

.dark .terminal-entry-status-error,
.dark .terminal-entry-status-cancelled {
  background: #3a211b;
  color: #f0a08b;
}

.terminal-entry-snippet {
  margin-top: 10px;
  max-height: 88px;
  overflow: hidden;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  border-radius: 12px;
  background: #f6f1e8;
  color: #3a3329;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace;
  font-size: 11px;
  line-height: 1.55;
  padding: 9px;
}

.terminal-entry-result {
  background: #f0f5ef;
}

.dark .terminal-entry-snippet {
  background: #1f1f1f;
  color: #ddd3c6;
}

.dark .terminal-entry-result {
  background: #1d2923;
}
</style>
