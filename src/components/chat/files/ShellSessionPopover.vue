<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import type { ToolExecutionEntry } from "../../../lib/chat-types";
import { emitToast } from "../../../lib/toast";
import {
  getShellSessionStatus,
  resetShellSessionForConversation,
  type ShellSessionStatus,
} from "../../../features/chat/services/chat-api";

const props = defineProps<{
  conversationId: string | null;
  refreshKey?: number;
  currentTurnToolEntries?: ToolExecutionEntry[];
}>();

const rootRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);
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

const hasRunningShellTool = computed(() =>
  (props.currentTurnToolEntries ?? []).some(
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

const badgeClass = computed(() => {
  if (hasRunningShellTool.value || status.value?.busy) {
    return "bg-[#fff3cf] text-[#8a5a05] dark:bg-[#3c3016] dark:text-[#ffd37a]";
  }
  if ((status.value?.backgroundCount ?? 0) > 0) {
    return "bg-[#e4f4ed] text-[#2f7553] dark:bg-[#173428] dark:text-[#90d8b4]";
  }
  return "bg-[#efe9dc] text-[#6f6657] dark:bg-[#3a342d] dark:text-[#d8d0c2]";
});

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
  } finally {
    isLoading.value = false;
  }
};

const togglePanel = () => {
  isOpen.value = !isOpen.value;
  if (isOpen.value) {
    void loadStatus();
  }
};

const closePanel = () => {
  isOpen.value = false;
};

const resetSession = async () => {
  if (!props.conversationId || isResetting.value) return;
  isResetting.value = true;
  try {
    await resetShellSessionForConversation(props.conversationId);
    await loadStatus();
    emitToast({ variant: "success", source: "shell-session", message: "终端会话已重置。" });
  } catch (error) {
    console.error("Failed to reset shell session:", error);
    emitToast({ variant: "error", source: "shell-session", message: "重置终端会话失败。" });
  } finally {
    isResetting.value = false;
  }
};

const onPointerDownDocument = (event: MouseEvent) => {
  if (!isOpen.value || !rootRef.value) return;
  const target = event.target as Node | null;
  if (target && !rootRef.value.contains(target)) {
    closePanel();
  }
};

watch(
  () => props.conversationId,
  () => {
    status.value = null;
    void loadStatus();
  },
);

watch(
  () => props.refreshKey,
  () => {
    void loadStatus();
  },
);

watch(
  () => hasRunningShellTool.value,
  (running) => {
    if (running) {
      void loadStatus();
    }
  },
);

onMounted(() => {
  document.addEventListener("mousedown", onPointerDownDocument);
  refreshTimer = setInterval(() => {
    if (isOpen.value || status.value?.busy || hasRunningShellTool.value) {
      void loadStatus();
    }
  }, 1500);
  void loadStatus();
});

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onPointerDownDocument);
  if (refreshTimer) {
    clearInterval(refreshTimer);
  }
});
</script>

<template>
  <div ref="rootRef" class="relative pointer-events-auto">
    <Button
      variant="outline"
      size="sm"
      class="h-8 px-3 rounded-md border border-[#e5e0d6] dark:border-[#444] bg-white/95 dark:bg-[#262626] text-[12px] text-[#5f584a] dark:text-[#d5cdc0] inline-flex items-center gap-2 hover:bg-[#f7f4ed] dark:hover:bg-[#2f2f2f] transition-colors"
      title="终端会话"
      @click="togglePanel"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="4 17 10 11 4 5" />
        <line x1="12" y1="19" x2="20" y2="19" />
      </svg>
      终端
      <span class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1.5 rounded-full text-[10px] leading-none" :class="badgeClass">
        {{ statusLabel }}
      </span>
    </Button>

    <div
      v-if="isOpen"
      class="absolute right-0 top-10 w-[360px] overflow-hidden rounded-2xl border border-[#e6e1d6] dark:border-[#464646] bg-white dark:bg-[#242424] shadow-[0_18px_56px_rgba(0,0,0,0.18)]"
    >
      <div class="px-3 py-2.5 border-b border-[#eee8dd] dark:border-[#3a3a3a] text-[12px] text-[#726957] dark:text-[#b9b1a6] flex items-center justify-between">
        <span class="font-medium">Shell session</span>
        <button type="button" class="text-[11px] hover:text-[#1f1a13] dark:hover:text-[#f2eee8]" @click="loadStatus">
          刷新
        </button>
      </div>

      <div class="p-3 space-y-3 text-[12px] text-[#5f584a] dark:text-[#d5cdc0]">
        <div v-if="!conversationId" class="text-[#9a9283]">当前还没有选中的会话。</div>
        <div v-else class="rounded-xl bg-[#faf7f0] dark:bg-[#2c2a27] border border-[#eee5d7] dark:border-[#3b3834] p-3 space-y-2">
          <div class="flex items-center justify-between">
            <span class="text-[#8b806f]">状态</span>
            <span class="font-medium">{{ isLoading && !status ? "读取中" : statusLabel }}</span>
          </div>
          <div class="flex items-start justify-between gap-3">
            <span class="text-[#8b806f] shrink-0">cwd</span>
            <code class="min-w-0 text-right break-all text-[11px] text-[#2f2a22] dark:text-[#f0ece5]">
              {{ status?.cwd || "尚未创建终端会话" }}
            </code>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-[#8b806f]">后台进程</span>
            <span>{{ status?.backgroundCount ?? 0 }}</span>
          </div>
          <div v-if="status?.backgroundPids?.length" class="text-[11px] text-[#8b806f] break-all">
            PID: {{ status.backgroundPids.join(", ") }}
          </div>
        </div>

        <p class="text-[11px] leading-relaxed text-[#8b806f] dark:text-[#aaa197]">
          这里展示的是 execute_bash / execute_powershell 复用的持久终端会话；普通 Sleep、MCP 等工具不会占用它。
        </p>

        <Button
          variant="outline"
          size="sm"
          class="w-full h-8 rounded-lg border-[#dfd5c5] text-[#7a3d2e] hover:bg-[#fff3ed] dark:border-[#4a3a33] dark:text-[#f0b3a0] dark:hover:bg-[#362722]"
          :disabled="!conversationId || isResetting"
          @click="resetSession"
        >
          {{ isResetting ? "正在重置..." : "重置终端会话" }}
        </Button>
      </div>
    </div>
  </div>
</template>
