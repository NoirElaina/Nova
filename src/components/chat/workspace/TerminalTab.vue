<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import type { ToolExecutionEntry } from "../../../lib/chat-types";
import {
  executeShellCommandForConversation,
  getShellSessionStatus,
  resetShellSessionForConversation,
  type ShellCommandResult,
  type ShellSessionStatus,
} from "../../../features/chat/services/chat-api";
import { emitToast } from "../../../lib/toast";

const props = defineProps<{
  conversationId: string | null;
  entries: ToolExecutionEntry[];
  currentTurnToolEntries?: ToolExecutionEntry[];
}>();

type TerminalSource = "agent" | "user";
type TerminalStatus = "running" | "completed" | "error" | "cancelled";

type TerminalEntry = {
  id: string;
  source: TerminalSource;
  command: string;
  result: string;
  stderr?: string;
  status: TerminalStatus;
  startedAt: number;
  finishedAt?: number;
  cwd?: string | null;
  exitCode?: number | null;
};

const commandText = ref("");
const isLoading = ref(false);
const isExecutingUserCommand = ref(false);
const status = ref<ShellSessionStatus | null>(null);
const localEntries = ref<TerminalEntry[]>([]);
const terminalBodyRef = ref<HTMLElement | null>(null);
const commandInputRef = ref<HTMLInputElement | null>(null);
const clearedAt = ref(0);
let refreshTimer: ReturnType<typeof setInterval> | undefined;

const isClearCommand = (command: string) =>
  ["cls", "clear", "clear-host"].includes(command.trim().toLowerCase());

const isShellToolName = (toolName: string) => {
  const normalized = toolName.trim().toLowerCase();
  return (
    normalized === "execute_bash" ||
    normalized === "execute_powershell" ||
    normalized === "reset_shell_session"
  );
};

const extractShellCommand = (entry: ToolExecutionEntry) => {
  if (entry.toolName.trim().toLowerCase() === "reset_shell_session") {
    return "reset_shell_session";
  }

  const input = entry.input.trim();
  if (!input) {
    return entry.toolName;
  }

  try {
    const parsed = JSON.parse(input) as { command?: unknown; cmd?: unknown; script?: unknown };
    const command = parsed.command ?? parsed.cmd ?? parsed.script;
    if (typeof command === "string" && command.trim()) {
      return command.trim();
    }
  } catch {
    // Tool input is sometimes already a plain command string.
  }

  return input;
};

const combinedAgentEntries = computed(() => {
  const byId = new Map<string, ToolExecutionEntry>();
  for (const entry of props.entries) {
    if (isShellToolName(entry.toolName)) {
      byId.set(entry.id, entry);
    }
  }
  for (const entry of props.currentTurnToolEntries ?? []) {
    if (isShellToolName(entry.toolName)) {
      byId.set(entry.id, entry);
    }
  }
  return Array.from(byId.values()).sort((a, b) => a.startedAt - b.startedAt);
});

const agentTerminalEntries = computed<TerminalEntry[]>(() =>
  combinedAgentEntries.value.map((entry) => ({
    id: `agent-${entry.id}`,
    source: "agent",
    command: extractShellCommand(entry),
    result: entry.result,
    status: entry.status,
    startedAt: entry.startedAt,
    finishedAt: entry.finishedAt,
  })),
);

const rawTerminalEntries = computed(() =>
  [...agentTerminalEntries.value, ...localEntries.value].sort((a, b) => a.startedAt - b.startedAt),
);

const terminalEntries = computed(() => {
  const latestClearAt = rawTerminalEntries.value.reduce(
    (latest, entry) => (isClearCommand(entry.command) ? Math.max(latest, entry.startedAt) : latest),
    clearedAt.value,
  );
  return rawTerminalEntries.value.filter(
    (entry) => entry.startedAt > latestClearAt && !isClearCommand(entry.command),
  );
});

const hasRunningEntry = computed(() =>
  terminalEntries.value.some((entry) => entry.status === "running"),
);

const currentCwd = computed(() => {
  const latestWithCwd = [...terminalEntries.value].reverse().find((entry) => entry.cwd);
  return latestWithCwd?.cwd || status.value?.cwd || "Nova";
});

const shellName = computed(() => {
  if (navigator.userAgent.toLowerCase().includes("windows")) {
    return "PowerShell";
  }
  return "Shell";
});

const tabTitle = computed(() => {
  const cwd = currentCwd.value;
  if (!cwd) return shellName.value;
  return cwd;
});

const terminalBanner = computed(() => {
  if (navigator.userAgent.toLowerCase().includes("windows")) {
    return ["PowerShell 7", "Copyright (c) Microsoft Corporation. 保留所有权利。"];
  }
  return ["Nova shell session"];
});

const formatPrompt = (cwd?: string | null) => `${cwd || currentCwd.value}>`;

const formatResult = (entry: TerminalEntry) => {
  if (entry.status === "running") {
    return "running...";
  }
  if (entry.status === "cancelled") {
    return "cancelled";
  }
  const result = entry.result.trim();
  const stderr = entry.stderr?.trim() ?? "";
  if (stderr && result) {
    return `${stderr}\n${result}`;
  }
  if (stderr) {
    return stderr;
  }
  if (result) {
    return result;
  }
  return "";
};

const scrollToBottom = async () => {
  await nextTick();
  const target = terminalBodyRef.value;
  if (target) {
    target.scrollTop = target.scrollHeight;
  }
};

const loadStatus = async () => {
  isLoading.value = true;
  try {
    status.value = await getShellSessionStatus(props.conversationId);
  } catch (error) {
    console.error("Failed to load shell session status:", error);
  } finally {
    isLoading.value = false;
  }
};

const applyCommandResult = (entryId: string, result: ShellCommandResult) => {
  const entry = localEntries.value.find((item) => item.id === entryId);
  if (!entry) {
    return;
  }

  entry.result = result.stdout;
  entry.stderr = result.stderr;
  entry.cwd = result.cwd;
  entry.exitCode = result.exitCode;
  entry.finishedAt = Date.now();

  if (result.cancelled) {
    entry.status = "cancelled";
  } else if (result.timedOut || (typeof result.exitCode === "number" && result.exitCode !== 0)) {
    entry.status = "error";
  } else {
    entry.status = "completed";
  }
};

const submitCommand = async () => {
  const command = commandText.value.trim();
  if (!command || isExecutingUserCommand.value) {
    return;
  }

  commandText.value = "";
  if (isClearCommand(command)) {
    clearedAt.value = Date.now();
    localEntries.value = [];
    await loadStatus();
    await scrollToBottom();
    commandInputRef.value?.focus();
    return;
  }

  const entryId = `user-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  localEntries.value.push({
    id: entryId,
    source: "user",
    command,
    result: "",
    status: "running",
    startedAt: Date.now(),
    cwd: currentCwd.value,
  });

  isExecutingUserCommand.value = true;
  await scrollToBottom();

  try {
    const result = await executeShellCommandForConversation(props.conversationId, command, {
      timeoutMs: 300_000,
    });
    applyCommandResult(entryId, result);
    await loadStatus();
  } catch (error) {
    const entry = localEntries.value.find((item) => item.id === entryId);
    if (entry) {
      entry.status = "error";
      entry.result = String(error);
      entry.finishedAt = Date.now();
    }
    emitToast({ variant: "error", source: "terminal-tab", message: "命令执行失败。" });
  } finally {
    isExecutingUserCommand.value = false;
    await scrollToBottom();
    commandInputRef.value?.focus();
  }
};

const resetSession = async () => {
  if (isExecutingUserCommand.value) {
    return;
  }

  try {
    await resetShellSessionForConversation(props.conversationId);
    localEntries.value.push({
      id: `system-${Date.now()}`,
      source: "user",
      command: "reset_shell_session",
      result: "session reset",
      status: "completed",
      startedAt: Date.now(),
      finishedAt: Date.now(),
    });
    await loadStatus();
    await scrollToBottom();
  } catch (error) {
    console.error("Failed to reset shell session:", error);
    emitToast({ variant: "error", source: "terminal-tab", message: "重置终端会话失败。" });
  }
};

watch(
  () => props.conversationId,
  () => {
    localEntries.value = [];
    clearedAt.value = 0;
    void loadStatus();
  },
  { immediate: true },
);

watch(
  () => terminalEntries.value.map((entry) => `${entry.id}:${entry.status}:${entry.result}`).join("|"),
  () => {
    void scrollToBottom();
  },
);

onMounted(() => {
  refreshTimer = setInterval(() => {
    if (hasRunningEntry.value || status.value?.busy) {
      void loadStatus();
    }
  }, 1500);
  void scrollToBottom();
});

onBeforeUnmount(() => {
  if (refreshTimer) {
    clearInterval(refreshTimer);
  }
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-white text-black dark:bg-[#1e1e1e] dark:text-[#f3f3f3]">
    <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 dark:border-[#333]">
      <div class="flex min-w-0 items-center gap-2">
        <div class="flex h-8 max-w-[220px] items-center gap-2 rounded-lg bg-[#f4f5f7] px-2.5 text-[13px] text-black dark:bg-[#2a2a2a] dark:text-[#f3f3f3]">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z" />
            <path d="m7 9 3 3-3 3" />
            <path d="M12 15h5" />
          </svg>
          <span class="truncate">{{ tabTitle }}</span>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f4f5f7] dark:hover:bg-[#2a2a2a]"
          title="重置终端会话"
          @click="resetSession"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 5v14" />
            <path d="M5 12h14" />
          </svg>
        </Button>
      </div>

      <div class="flex items-center gap-1">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f4f5f7] dark:hover:bg-[#2a2a2a]"
          title="刷新状态"
          :disabled="isLoading"
          @click="loadStatus"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
            <path d="M3 21v-5h5" />
            <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
            <path d="M16 8h5V3" />
          </svg>
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f4f5f7] dark:hover:bg-[#2a2a2a]"
          title="聚焦输入"
          @click="commandInputRef?.focus()"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M8 3H5a2 2 0 0 0-2 2v3" />
            <path d="M16 3h3a2 2 0 0 1 2 2v3" />
            <path d="M8 21H5a2 2 0 0 1-2-2v-3" />
            <path d="M16 21h3a2 2 0 0 0 2-2v-3" />
          </svg>
        </Button>
      </div>
    </div>

    <div
      ref="terminalBodyRef"
      class="min-h-0 flex-1 overflow-y-auto px-4 py-2 font-mono text-[13px] leading-[1.45] text-black dark:text-[#f3f3f3]"
      @click="commandInputRef?.focus()"
    >
      <div class="mb-5 whitespace-pre-wrap">
        <div v-for="line in terminalBanner" :key="line">{{ line }}</div>
      </div>

      <div v-for="entry in terminalEntries" :key="entry.id" class="mb-5 whitespace-pre-wrap break-words">
        <div class="whitespace-pre-wrap break-words text-black dark:text-[#f3f3f3]">
          <span class="text-[#6b7280] dark:text-[#9ca3af]">{{ formatPrompt(entry.cwd) }}</span>{{ entry.command }}
        </div>
        <div
          v-if="formatResult(entry)"
          class="mt-1 whitespace-pre-wrap break-words pl-0"
          :class="entry.status === 'error' ? 'text-[#b91c1c] dark:text-[#fca5a5]' : 'text-black dark:text-[#f3f3f3]'"
        >
          {{ formatResult(entry) }}
        </div>
      </div>

      <form
        class="mt-1 flex min-w-0 items-center gap-0"
        @submit.prevent="submitCommand"
        @click.stop
      >
        <span class="shrink-0 text-[#6b7280] dark:text-[#9ca3af]">{{ formatPrompt(currentCwd) }}</span>
        <input
          ref="commandInputRef"
          v-model="commandText"
          class="min-w-0 flex-1 bg-transparent pl-0.5 text-black outline-none dark:text-[#f3f3f3]"
          :disabled="isExecutingUserCommand"
          placeholder=""
          spellcheck="false"
        />
        <span v-if="isExecutingUserCommand" class="shrink-0 font-sans text-[12px] text-[#64748b] dark:text-[#aaa]">执行中</span>
      </form>
    </div>
  </div>
</template>
