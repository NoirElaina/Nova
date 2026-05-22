<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import type { ToolExecutionEntry } from "../../../lib/chat-types";
import {
  getShellSessionStatus,
  resetShellSessionForConversation,
  type ShellSessionStatus,
} from "../../../features/chat/services/chat-api";
import { emitToast } from "../../../lib/toast";

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
    normalized === "execute_bash" ||
    normalized === "execute_powershell" ||
    normalized === "reset_shell_session"
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
    (entry) => entry.status === "running" && isShellToolName(entry.toolName),
  ),
);

const statusLabel = computed(() => {
  if (hasRunningShellTool.value) return "执行中";
  if (!status.value?.exists) return "未启动";
  if (status.value.busy) return "执行中";
  if (!status.value.alive) return "已断开";
  if (status.value.backgroundCount > 0) return `后台 ${status.value.backgroundCount}`;
  return "空闲";
});

const statusPillClass = computed(() => {
  if (hasRunningShellTool.value || status.value?.busy) {
    return "bg-[#fff7ed] text-[#c2410c] dark:bg-[#3b2618] dark:text-[#fdba74]";
  }
  if (!status.value?.exists) {
    return "bg-[#f3f4f6] text-[#6b7280] dark:bg-[#2d2d2d] dark:text-[#bdbdbd]";
  }
  if (!status.value.alive) {
    return "bg-[#fef2f2] text-[#dc2626] dark:bg-[#3b1f1f] dark:text-[#fca5a5]";
  }
  if (status.value.backgroundCount > 0) {
    return "bg-[#eff6ff] text-[#2563eb] dark:bg-[#172554] dark:text-[#93c5fd]";
  }
  return "bg-[#ecfdf5] text-[#059669] dark:bg-[#123225] dark:text-[#86efac]";
});

const currentCwd = computed(() => status.value?.cwd || "尚未创建终端会话");
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
    console.error("Failed to load shell session status:", error);
    emitToast({ variant: "error", source: "terminal-tab", message: "读取终端状态失败。" });
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
    emitToast({ variant: "success", source: "terminal-tab", message: "终端会话已重置。" });
  } catch (error) {
    console.error("Failed to reset shell session:", error);
    emitToast({ variant: "error", source: "terminal-tab", message: "重置终端会话失败。" });
  } finally {
    isResetting.value = false;
  }
};

const formatTime = (timestamp: number) =>
  new Date(timestamp).toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });

const formatDuration = (entry: ToolExecutionEntry) => {
  if (!entry.finishedAt) return "运行中";
  const duration = Math.max(0, entry.finishedAt - entry.startedAt);
  return duration >= 1000 ? `${(duration / 1000).toFixed(1)}s` : `${duration}ms`;
};

const entryStatusLabel = (statusValue: ToolExecutionEntry["status"]) => {
  if (statusValue === "completed") return "完成";
  if (statusValue === "error") return "失败";
  if (statusValue === "cancelled") return "取消";
  return "运行中";
};

const entryStatusClass = (statusValue: ToolExecutionEntry["status"]) => {
  if (statusValue === "completed") {
    return "bg-[#ecfdf5] text-[#059669] dark:bg-[#123225] dark:text-[#86efac]";
  }
  if (statusValue === "error" || statusValue === "cancelled") {
    return "bg-[#fef2f2] text-[#dc2626] dark:bg-[#3b1f1f] dark:text-[#fca5a5]";
  }
  return "bg-[#fff7ed] text-[#c2410c] dark:bg-[#3b2618] dark:text-[#fdba74]";
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
  <div class="h-full overflow-y-auto bg-white px-4 py-4 dark:bg-[#1e1e1e]">
    <div class="flex items-start justify-between gap-4 rounded-xl border border-[#e5e7eb] bg-white p-4 dark:border-[#333] dark:bg-[#252525]">
      <div class="min-w-0">
        <div class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[#6b7280] dark:text-[#aaa]">
          Shell Session
        </div>
        <div class="mt-1 text-xl font-semibold tracking-[-0.02em] text-[#111827] dark:text-[#f2f2f2]">
          终端
        </div>
        <p class="mt-1 max-w-[520px] text-[13px] leading-6 text-[#6b7280] dark:text-[#aaa]">
          展示 agent 的 execute_bash / execute_powershell 持久会话状态。
        </p>
      </div>

      <div class="flex flex-wrap justify-end gap-2">
        <span
          class="inline-flex h-8 items-center rounded-full px-3 text-xs font-medium"
          :class="statusPillClass"
        >
          {{ isLoading && !status ? "读取中" : statusLabel }}
        </span>
        <Button
          type="button"
          variant="outline"
          size="sm"
          class="h-8 rounded-lg border-[#e5e7eb] bg-white text-[#4b5563] hover:bg-[#f3f4f6] dark:border-[#3a3a3a] dark:bg-[#252525] dark:text-[#ddd] dark:hover:bg-[#303030]"
          :disabled="isLoading"
          @click="loadStatus"
        >
          {{ isLoading ? "刷新中" : "刷新" }}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          class="h-8 rounded-lg border-[#e5e7eb] bg-white text-[#b91c1c] hover:bg-[#fef2f2] dark:border-[#3a3a3a] dark:bg-[#252525] dark:text-[#fca5a5] dark:hover:bg-[#3b1f1f]"
          :disabled="!conversationId || isResetting"
          @click="resetSession"
        >
          {{ isResetting ? "重置中" : "重置会话" }}
        </Button>
      </div>
    </div>

    <div class="mt-3 grid grid-cols-2 gap-3">
      <div class="rounded-xl border border-[#e5e7eb] bg-white p-3 dark:border-[#333] dark:bg-[#252525]">
        <div class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[#6b7280] dark:text-[#aaa]">状态</div>
        <div class="mt-2 text-xl font-semibold text-[#111827] dark:text-[#f2f2f2]">{{ statusLabel }}</div>
      </div>
      <div class="rounded-xl border border-[#e5e7eb] bg-white p-3 dark:border-[#333] dark:bg-[#252525]">
        <div class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[#6b7280] dark:text-[#aaa]">后台进程</div>
        <div class="mt-2 text-xl font-semibold text-[#111827] dark:text-[#f2f2f2]">{{ status?.backgroundCount ?? 0 }}</div>
      </div>
      <div class="col-span-2 rounded-xl border border-[#e5e7eb] bg-white p-3 dark:border-[#333] dark:bg-[#252525]">
        <div class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[#6b7280] dark:text-[#aaa]">当前目录</div>
        <code class="mt-2 block break-all text-xs leading-6 text-[#374151] dark:text-[#ddd]">{{ currentCwd }}</code>
      </div>
      <div class="col-span-2 rounded-xl border border-[#e5e7eb] bg-white p-3 dark:border-[#333] dark:bg-[#252525]">
        <div class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[#6b7280] dark:text-[#aaa]">会话</div>
        <code class="mt-2 block break-all text-xs leading-6 text-[#374151] dark:text-[#ddd]">
          {{ conversationId || "当前还没有选中的会话" }}
        </code>
      </div>
    </div>

    <div
      v-if="backgroundPids.length"
      class="mt-3 rounded-xl border border-[#e5e7eb] bg-white p-3 dark:border-[#333] dark:bg-[#252525]"
    >
      <div class="text-sm font-semibold text-[#111827] dark:text-[#f2f2f2]">后台 PID</div>
      <div class="mt-2 flex flex-wrap gap-2">
        <span
          v-for="pid in backgroundPids"
          :key="pid"
          class="rounded-full bg-[#f3f4f6] px-2.5 py-1 text-xs text-[#4b5563] dark:bg-[#303030] dark:text-[#ddd]"
        >
          {{ pid }}
        </span>
      </div>
    </div>

    <div class="mt-3 rounded-xl border border-[#e5e7eb] bg-white p-3 dark:border-[#333] dark:bg-[#252525]">
      <div class="flex items-start justify-between gap-3">
        <div>
          <div class="text-sm font-semibold text-[#111827] dark:text-[#f2f2f2]">最近终端工具</div>
          <div class="mt-1 text-xs text-[#6b7280] dark:text-[#aaa]">最近 10 条 shell 工具记录。</div>
        </div>
        <span class="inline-flex h-6 min-w-7 items-center justify-center rounded-full bg-[#f3f4f6] px-2 text-xs font-medium text-[#6b7280] dark:bg-[#303030] dark:text-[#ddd]">
          {{ shellEntries.length }}
        </span>
      </div>

      <div v-if="shellEntries.length === 0" class="mt-3 rounded-lg border border-dashed border-[#e5e7eb] px-3 py-5 text-center text-sm text-[#6b7280] dark:border-[#444] dark:text-[#aaa]">
        还没有终端工具调用。
      </div>

      <div v-else class="mt-3 grid gap-2">
        <article
          v-for="entry in shellEntries"
          :key="entry.id"
          class="rounded-lg border border-[#edf0f3] bg-[#fcfcfd] p-3 dark:border-[#333] dark:bg-[#292929]"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="truncate text-sm font-medium text-[#111827] dark:text-[#f2f2f2]">{{ entry.toolName }}</div>
              <div class="mt-1 text-[11px] text-[#6b7280] dark:text-[#aaa]">
                {{ formatTime(entry.startedAt) }} · {{ formatDuration(entry) }}
              </div>
            </div>
            <span
              class="inline-flex h-6 shrink-0 items-center rounded-full px-2 text-xs font-medium"
              :class="entryStatusClass(entry.status)"
            >
              {{ entryStatusLabel(entry.status) }}
            </span>
          </div>
          <pre v-if="entry.input" class="mt-2 max-h-20 overflow-hidden whitespace-pre-wrap break-words rounded-md bg-[#f3f4f6] p-2 font-mono text-[11px] leading-5 text-[#374151] dark:bg-[#1f1f1f] dark:text-[#ddd]">{{ entry.input }}</pre>
          <pre v-if="entry.result" class="mt-2 max-h-20 overflow-hidden whitespace-pre-wrap break-words rounded-md bg-[#f8fafc] p-2 font-mono text-[11px] leading-5 text-[#374151] dark:bg-[#202626] dark:text-[#ddd]">{{ entry.result }}</pre>
        </article>
      </div>
    </div>
  </div>
</template>
