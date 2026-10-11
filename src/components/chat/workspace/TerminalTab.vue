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
import { emitToast } from "../../../lib/toast";

const USER_TERMINAL_EVENT = "user-terminal-output";

const props = defineProps<{
  conversationId: string | null;
  visible: boolean;
}>();

const terminalHostRef = ref<HTMLDivElement | null>(null);
const sessionId = ref<string | null>(null);
const cwd = ref("workspace");
const isStarting = ref(false);
const error = ref("");
const activeConversationKey = ref<string | null>(null);

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
  if (!terminal || !fitAddon || !terminalHostRef.value || !props.visible) return;
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
  if (!props.visible || isStarting.value) return;
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
    if (!visible) return;
    await nextTick();
    scheduleResize();
    terminal?.scrollToBottom();
    terminal?.focus();
  },
);

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
  <div class="flex h-full min-h-0 flex-col text-black dark:text-[#f3f3f3]">
    <!-- 工具栏：左侧终端标签，右侧快捷操作 -->
    <div class="flex h-10 shrink-0 items-center justify-between border-b border-[#eef0f3] px-2.5 dark:border-[#2c2c2c]">
      <div class="flex min-w-0 items-center gap-1.5">
        <div class="flex h-7 items-center gap-1.5 rounded-md bg-[#eef2f7] px-2 text-[12px] font-medium text-[#111827] dark:bg-white/10 dark:text-[#ececec]">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" class="text-[#64748b] dark:text-[#9ca3af]">
            <path d="M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1Z" />
            <path d="m7 9 3 3-3 3" />
            <path d="M12 15h5" />
          </svg>
          <span class="max-w-[200px] truncate font-mono text-[11px]">{{ tabTitle }}</span>
        </div>
      </div>

      <div class="flex items-center gap-1">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]"
          title="重启终端"
          :disabled="isStarting"
          @click="restartTerminal"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12a9 9 0 1 1-2.64-6.36" />
            <path d="M21 3v6h-6" />
          </svg>
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]"
          title="滚动到底部"
          @click="terminal?.scrollToBottom()"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 5v14" />
            <path d="m19 12-7 7-7-7" />
          </svg>
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]"
          title="适配尺寸"
          @click="scheduleResize"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
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
      class="min-h-0 flex-1 overflow-hidden px-2 py-1"
      @click="terminal?.focus()"
    />
  </div>
</template>
