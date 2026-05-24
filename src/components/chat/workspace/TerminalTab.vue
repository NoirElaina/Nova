<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { Button } from "@/components/ui/button";
import {
  resizeUserTerminal,
  startUserTerminal,
  stopUserTerminal,
  writeUserTerminal,
  type UserTerminalOutputEvent,
} from "../../../features/chat/services/chat-api";
import type { ToolExecutionEntry } from "../../../lib/chat-types";
import { emitToast } from "../../../lib/toast";

const USER_TERMINAL_EVENT = "user-terminal-output";

const props = defineProps<{
  conversationId: string | null;
  visible: boolean;
  entries: ToolExecutionEntry[];
  currentTurnToolEntries?: ToolExecutionEntry[];
}>();

type TerminalPane = "user" | "agent";

const terminalHostRef = ref<HTMLDivElement | null>(null);
const sessionId = ref<string | null>(null);
const cwd = ref("workspace");
const isStarting = ref(false);
const error = ref("");
const activeConversationKey = ref<string | null>(null);
const activePane = ref<TerminalPane>("user");

let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let unlistenTerminal: UnlistenFn | null = null;
let resizeObserver: ResizeObserver | null = null;
let inputDisposable: { dispose: () => void } | null = null;
let resizeTimer: number | null = null;

const tabTitle = computed(() => {
  const parts = cwd.value.split(/[\\/]/).filter(Boolean);
  const tail = parts.length ? parts[parts.length - 1] : "";
  return tail || cwd.value || "workspace";
});

const normalizedConversationId = () => props.conversationId?.trim() || null;

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
  if (!input) return entry.toolName;

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

const formatShellResult = (entry: ToolExecutionEntry) => {
  if (entry.status === "running") return "running...";
  if (entry.status === "cancelled") return "cancelled";

  const raw = entry.result.trim();
  if (!raw) return "";

  try {
    const parsed = JSON.parse(raw) as {
      stdout?: unknown;
      stderr?: unknown;
      output?: unknown;
      error?: unknown;
    };
    const stderr = typeof parsed.stderr === "string" ? parsed.stderr.trim() : "";
    const stdout =
      typeof parsed.stdout === "string"
        ? parsed.stdout.trim()
        : typeof parsed.output === "string"
          ? parsed.output.trim()
          : "";
    const error = typeof parsed.error === "string" ? parsed.error.trim() : "";
    return [stderr, stdout, error].filter(Boolean).join("\n");
  } catch {
    return raw;
  }
};

const aiTerminalEntries = computed(() => {
  const byId = new Map<string, ToolExecutionEntry>();
  for (const entry of props.entries) {
    if (isShellToolName(entry.toolName)) byId.set(entry.id, entry);
  }
  for (const entry of props.currentTurnToolEntries ?? []) {
    if (isShellToolName(entry.toolName)) byId.set(entry.id, entry);
  }
  return Array.from(byId.values()).sort((a, b) => a.startedAt - b.startedAt);
});

const aiTerminalCount = computed(() => aiTerminalEntries.value.length);

const switchPane = (pane: TerminalPane) => {
  activePane.value = pane;
  if (pane === "user") {
    void startOrAttach();
    void nextTick(() => {
      scheduleResize();
      terminal?.focus();
    });
  }
};

const createTerminal = async () => {
  if (terminal || !terminalHostRef.value) return;

  terminal = new Terminal({
    cursorBlink: true,
    fontFamily: '"Cascadia Mono", "JetBrains Mono", Consolas, monospace',
    fontSize: 13,
    lineHeight: 1.18,
    scrollback: 6000,
    convertEol: false,
    theme: {
      background: "#ffffff",
      foreground: "#111827",
      cursor: "#111827",
      selectionBackground: "#dbeafe",
      black: "#111827",
      blue: "#2563eb",
      cyan: "#0891b2",
      green: "#059669",
      magenta: "#7c3aed",
      red: "#dc2626",
      white: "#e5e7eb",
      yellow: "#92400e",
      brightBlack: "#64748b",
      brightBlue: "#1d4ed8",
      brightCyan: "#0e7490",
      brightGreen: "#047857",
      brightMagenta: "#6d28d9",
      brightRed: "#b91c1c",
      brightWhite: "#111827",
      brightYellow: "#78350f",
    },
  });

  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(terminalHostRef.value);

  inputDisposable = terminal.onData((data) => {
    if (!sessionId.value) return;
    void writeUserTerminal(normalizedConversationId(), data).catch((err) => {
      error.value = String(err);
    });
  });

  resizeObserver = new ResizeObserver(() => scheduleResize());
  resizeObserver.observe(terminalHostRef.value);
  await nextTick();
  scheduleResize();
};

const terminalSize = () => ({
  rows: terminal?.rows ?? 24,
  cols: terminal?.cols ?? 80,
});

const fitTerminal = () => {
  if (!terminal || !fitAddon || !terminalHostRef.value || !props.visible || activePane.value !== "user") return;
  try {
    fitAddon.fit();
    terminal.scrollToBottom();
  } catch (err) {
    console.warn("Failed to fit terminal:", err);
  }
};

const scheduleResize = () => {
  if (resizeTimer !== null) {
    window.clearTimeout(resizeTimer);
  }
  resizeTimer = window.setTimeout(() => {
    resizeTimer = null;
    fitTerminal();
    if (sessionId.value) {
      void resizeUserTerminal(normalizedConversationId(), terminalSize()).catch((err) => {
        console.warn("Failed to resize terminal:", err);
      });
    }
  }, 80);
};

const handleTerminalEvent = (payload: UserTerminalOutputEvent) => {
  if (!terminal) return;
  const sameConversation = (payload.conversationId?.trim() || null) === normalizedConversationId();
  if (payload.sessionId !== sessionId.value) {
    if (!sessionId.value && sameConversation) {
      sessionId.value = payload.sessionId;
    } else {
      return;
    }
  }

  if (payload.kind === "output") {
    terminal.write(payload.data ?? "");
    terminal.scrollToBottom();
    return;
  }

  if (payload.kind === "error") {
    terminal.writeln(`\r\n[terminal error] ${payload.error ?? "unknown error"}`);
    terminal.scrollToBottom();
    return;
  }

  if (payload.kind === "exit") {
    terminal.writeln("\r\n[terminal exited]");
    terminal.scrollToBottom();
    sessionId.value = null;
  }
};

const ensureEventListener = async () => {
  if (unlistenTerminal) return;
  unlistenTerminal = await listen<UserTerminalOutputEvent>(USER_TERMINAL_EVENT, (event) => {
    handleTerminalEvent(event.payload);
  });
};

const startOrAttach = async () => {
  if (!props.visible || activePane.value !== "user" || isStarting.value) return;
  isStarting.value = true;
  error.value = "";

  try {
    await nextTick();
    if (!terminalHostRef.value) return;
    await createTerminal();
    await ensureEventListener();
    fitTerminal();
    const info = await startUserTerminal(normalizedConversationId(), terminalSize());
    sessionId.value = info.sessionId;
    cwd.value = info.cwd;
    await nextTick();
    scheduleResize();
    terminal?.scrollToBottom();
    terminal?.focus();
  } catch (err) {
    error.value = String(err);
    emitToast({ variant: "error", source: "terminal-tab", message: "启动终端失败。" });
  } finally {
    isStarting.value = false;
  }
};

const restartTerminal = async () => {
  if (isStarting.value) return;
  try {
    await stopUserTerminal(normalizedConversationId());
    sessionId.value = null;
    terminal?.clear();
    terminal?.reset();
    await startOrAttach();
  } catch (err) {
    error.value = String(err);
    emitToast({ variant: "error", source: "terminal-tab", message: "重置终端失败。" });
  }
};

watch(
  () => [props.visible, props.conversationId] as const,
  async ([visible]) => {
    if (!visible) return;

    const nextConversationKey = normalizedConversationId();
    if (activeConversationKey.value !== nextConversationKey) {
      terminal?.clear();
      terminal?.reset();
      sessionId.value = null;
      activeConversationKey.value = nextConversationKey;
    }

    await startOrAttach();
  },
  { immediate: true },
);

watch(
  () => props.visible,
  async (visible) => {
    if (!visible || activePane.value !== "user") return;
    await nextTick();
    scheduleResize();
    terminal?.scrollToBottom();
    terminal?.focus();
  },
);

watch(activePane, async (pane) => {
  if (pane !== "user" || !props.visible) return;
  await nextTick();
  await startOrAttach();
  scheduleResize();
  terminal?.scrollToBottom();
});

onBeforeUnmount(() => {
  if (resizeTimer !== null) {
    window.clearTimeout(resizeTimer);
  }
  resizeObserver?.disconnect();
  inputDisposable?.dispose();
  terminal?.dispose();
  if (unlistenTerminal) {
    void Promise.resolve(unlistenTerminal()).catch((err) => {
      console.warn("Failed to remove terminal listener:", err);
    });
  }
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-white text-black dark:bg-[#1e1e1e] dark:text-[#f3f3f3]">
    <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 dark:border-[#333]">
      <div class="flex min-w-0 items-center gap-2">
        <button
          type="button"
          class="flex h-8 max-w-[220px] items-center gap-2 rounded-lg px-2.5 text-[13px] transition-colors"
          :class="activePane === 'user'
            ? 'bg-[#f4f5f7] text-black dark:bg-[#2a2a2a] dark:text-[#f3f3f3]'
            : 'text-[#64748b] hover:bg-[#f8f9fa] dark:text-[#94a3b8] dark:hover:bg-[#2a2a2a]'"
          @click="switchPane('user')"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z" />
            <path d="m7 9 3 3-3 3" />
            <path d="M12 15h5" />
          </svg>
          <span class="truncate">{{ tabTitle }}</span>
        </button>
        <button
          type="button"
          class="flex h-8 items-center gap-2 rounded-lg px-2.5 text-[13px] transition-colors"
          :class="activePane === 'agent'
            ? 'bg-[#f4f5f7] text-black dark:bg-[#2a2a2a] dark:text-[#f3f3f3]'
            : 'text-[#64748b] hover:bg-[#f8f9fa] dark:text-[#94a3b8] dark:hover:bg-[#2a2a2a]'"
          @click="switchPane('agent')"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M7 8h10" />
            <path d="M7 12h6" />
            <path d="M5 4h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H8l-4 3v-3H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z" />
          </svg>
          <span>AI 终端</span>
          <span class="rounded-full bg-black/5 px-1.5 py-0.5 text-[11px] text-[#64748b] dark:bg-white/10 dark:text-[#cbd5e1]">{{ aiTerminalCount }}</span>
        </button>
        <Button
          v-if="activePane === 'user'"
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f4f5f7] dark:hover:bg-[#2a2a2a]"
          title="重启终端"
          :disabled="isStarting"
          @click="restartTerminal"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 5v14" />
            <path d="M5 12h14" />
          </svg>
        </Button>
        <Button
          v-if="activePane === 'user'"
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f4f5f7] dark:hover:bg-[#2a2a2a]"
          title="滚动到底部"
          @click="terminal?.scrollToBottom()"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 5v14" />
            <path d="m19 12-7 7-7-7" />
          </svg>
        </Button>
      </div>

      <div class="flex items-center gap-1">
        <Button
          v-if="activePane === 'user'"
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f4f5f7] dark:hover:bg-[#2a2a2a]"
          title="适配尺寸"
          @click="scheduleResize"
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

    <div v-if="error" class="border-b border-rose-100 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:border-rose-950/50 dark:bg-rose-950/30 dark:text-rose-200">
      {{ error }}
    </div>

    <div
      ref="terminalHostRef"
      v-show="activePane === 'user'"
      class="min-h-0 flex-1 overflow-hidden px-2 py-1"
      @click="terminal?.focus()"
    />

    <div
      v-show="activePane === 'agent'"
      class="min-h-0 flex-1 overflow-y-auto bg-white px-4 py-3 font-mono text-[13px] leading-[1.5] text-[#111827] dark:bg-[#1e1e1e] dark:text-[#f3f3f3]"
    >
      <div v-if="aiTerminalEntries.length === 0" class="flex h-full items-center justify-center text-center font-sans text-sm text-[#64748b] dark:text-[#94a3b8]">
        AI 还没有执行终端命令。
      </div>
      <div v-else class="space-y-5">
        <section
          v-for="entry in aiTerminalEntries"
          :key="entry.id"
          class="whitespace-pre-wrap break-words"
        >
          <div class="flex items-center gap-2 font-sans text-xs text-[#64748b] dark:text-[#94a3b8]">
            <span class="rounded-md bg-[#f4f5f7] px-1.5 py-0.5 dark:bg-[#2a2a2a]">{{ entry.toolName }}</span>
            <span>{{ entry.status === 'running' ? '执行中' : entry.status === 'completed' ? '已完成' : entry.status === 'cancelled' ? '已取消' : '失败' }}</span>
          </div>
          <div class="mt-1">
            <span class="text-[#64748b] dark:text-[#9ca3af]">AI&gt;</span>
            <span>{{ extractShellCommand(entry) }}</span>
          </div>
          <div
            v-if="formatShellResult(entry)"
            class="mt-1 border-l border-[#e5e7eb] pl-3 dark:border-[#333]"
            :class="entry.status === 'error' ? 'text-[#b91c1c] dark:text-[#fca5a5]' : 'text-[#111827] dark:text-[#f3f3f3]'"
          >
            {{ formatShellResult(entry) }}
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
