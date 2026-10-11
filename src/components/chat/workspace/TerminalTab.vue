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

const LIGHT_THEME = {
  background: "#ffffff",
  foreground: "#0f172a",
  cursor: "#0f172a",
  cursorAccent: "#ffffff",
  selectionBackground: "#cbd5e1",
  black: "#0f172a",
  blue: "#2563eb",
  cyan: "#0891b2",
  green: "#059669",
  magenta: "#7c3aed",
  red: "#dc2626",
  white: "#f1f5f9",
  yellow: "#d97706",
  brightBlack: "#64748b",
  brightBlue: "#3b82f6",
  brightCyan: "#06b6d4",
  brightGreen: "#10b981",
  brightMagenta: "#8b5cf6",
  brightRed: "#ef4444",
  brightWhite: "#0f172a",
  brightYellow: "#f59e0b",
};

const DARK_THEME = {
  background: "#18181b",
  foreground: "#f4f4f5",
  cursor: "#f4f4f5",
  cursorAccent: "#18181b",
  selectionBackground: "#3f3f46",
  black: "#27272a",
  blue: "#60a5fa",
  cyan: "#38bdf8",
  green: "#34d399",
  magenta: "#a78bfa",
  red: "#f87171",
  white: "#e4e4e7",
  yellow: "#fbbf24",
  brightBlack: "#71717a",
  brightBlue: "#93c5fd",
  brightCyan: "#7dd3fc",
  brightGreen: "#6ee7b7",
  brightMagenta: "#c4b5fd",
  brightRed: "#fca5a5",
  brightWhite: "#ffffff",
  brightYellow: "#fde68a",
};

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
let themeObserver: MutationObserver | null = null;
let inputDisposable: { dispose: () => void } | null = null;
let resizeTimer: number | null = null;

const tabTitle = computed(() => {
  const parts = cwd.value.split(/[\\/]/).filter(Boolean);
  const tail = parts.length ? parts[parts.length - 1] : "";
  return tail || cwd.value || "workspace";
});

const isDark = () => document.documentElement.classList.contains("dark");
const getTerminalTheme = () => (isDark() ? DARK_THEME : LIGHT_THEME);

const normalizedConversationId = () => props.conversationId?.trim() || null;

const pasteFromClipboard = async () => {
  try {
    const text = await navigator.clipboard.readText();
    if (text && sessionId.value) {
      await writeUserTerminal(normalizedConversationId(), text);
    }
  } catch (err) {
    console.warn("无法读取剪贴板:", err);
  }
};

const handleContextMenu = async (event: MouseEvent) => {
  event.preventDefault();
  await pasteFromClipboard();
};

const createTerminal = async () => {
  if (terminal || !terminalHostRef.value) return;

  terminal = new Terminal({
    cursorBlink: true,
    fontFamily: '"Cascadia Mono", "JetBrains Mono", Consolas, monospace',
    fontSize: 13,
    lineHeight: 1.2,
    scrollback: 8000,
    convertEol: false,
    theme: getTerminalTheme(),
  });

  // 支持智能复制 (Ctrl+C 在有选区时复制文本，无选区时发送中断)
  terminal.attachCustomKeyEventHandler((event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c" && event.type === "keydown") {
      if (terminal?.hasSelection()) {
        const selection = terminal.getSelection();
        if (selection) {
          void navigator.clipboard.writeText(selection);
          terminal.clearSelection();
          return false;
        }
      }
    }
    // Ctrl+V 粘贴
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "v" && event.type === "keydown") {
      void pasteFromClipboard();
      return false;
    }
    return true;
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

  // 监听主题变化并动态切换 xterm 配色
  themeObserver = new MutationObserver(() => {
    if (terminal) {
      terminal.options.theme = getTerminalTheme();
    }
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });

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
  themeObserver?.disconnect();
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
  <div class="flex h-full min-h-0 flex-col bg-white text-[#111827] dark:bg-[#18181b] dark:text-[#f3f3f3]">
    <!-- 工具栏：左侧终端标签，右侧快捷操作 -->
    <div class="flex h-10 shrink-0 items-center justify-between border-b border-[#eef0f3] px-2.5 dark:border-[#27272a]">
      <div class="flex min-w-0 items-center gap-1.5">
        <div class="flex h-7 items-center gap-1.5 rounded-md bg-[#f1f5f9] px-2 text-[12px] font-medium text-[#111827] dark:bg-[#27272a] dark:text-[#ececec]">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" class="text-[#64748b] dark:text-[#a1a1aa]">
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
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
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
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
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
          class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
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

    <!-- 终端挂载容器：右键智能粘贴，点击聚焦，背景主题自适应 -->
    <div
      ref="terminalHostRef"
      class="min-h-0 flex-1 overflow-hidden bg-white px-2 py-1 dark:bg-[#18181b]"
      @click="terminal?.focus()"
      @contextmenu="handleContextMenu"
    />
  </div>
</template>
