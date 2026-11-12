<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch, type ComponentPublicInstance } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { Button } from "@/components/ui/button";
import {
  ArrowDownToLine,
  Check,
  Copy,
  Folder,
  Maximize2,
  PanelRight,
  Plus,
  RotateCw,
  Square,
  SquareTerminal,
  X,
} from "lucide-vue-next";
import {
  resizeUserTerminal,
  startUserTerminal,
  stopUserTerminal,
  writeUserTerminal,
  type UserTerminalOutputEvent,
} from "../../../features/chat/services/chat-api";
import { emitToast } from "../../../lib/toast";
import type { BackgroundOutput } from "@/lib/protocol";

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
  focusBgRequestKey?: number;
}>();

type TerminalInstanceState = {
  id: string;
  label: string;
  sessionId: string | null;
  cwd: string;
  isStarting: boolean;
  alive: boolean;
  error: string;
};

type ActiveSelection =
  | { kind: "terminal"; id: string }
  | { kind: "job"; id: string };

type RuntimeTerminalHandle = {
  terminal: Terminal;
  fitAddon: FitAddon;
  resizeObserver: ResizeObserver;
  inputDisposable: { dispose: () => void };
  resizeTimer: number | null;
};

const sidebarOpen = ref(true);
const terminals = ref<TerminalInstanceState[]>([
  {
    id: "main",
    label: "终端",
    sessionId: null,
    cwd: "workspace",
    isStarting: false,
    alive: false,
    error: "",
  },
]);
const selection = ref<ActiveSelection>({ kind: "terminal", id: "main" });

const hostElements = new Map<string, HTMLElement>();
const runtimes = new Map<string, RuntimeTerminalHandle>();
let terminalSeq = 1;
let unlistenTerminal: UnlistenFn | null = null;
let themeObserver: MutationObserver | null = null;
let activeConversationKey = ref<string | null>(null);

// 后台任务状态
const jobs = ref<BackgroundOutput[]>([]);
const jobsSyncedAt = ref(Date.now());
const nowTick = ref(Date.now());
const output = ref<BackgroundOutput | null>(null);
const loadingOutput = ref(false);
const outputError = ref("");
const killingId = ref<string | null>(null);
const autoScroll = ref(true);
const outputEl = ref<HTMLElement | null>(null);
const copiedCmd = ref(false);
const copiedCwd = ref(false);
const copiedOutput = ref(false);

let pollTimer: ReturnType<typeof setInterval> | null = null;
let clockTimer: ReturnType<typeof setInterval> | null = null;
let jobsRequestSeq = 0;
let outputRequestSeq = 0;
let lastHandledFocusKey = 0;

const normalizedConversationId = () => props.conversationId?.trim() || null;
const isDark = () => document.documentElement.classList.contains("dark");
const getTerminalTheme = () => (isDark() ? DARK_THEME : LIGHT_THEME);

const activeTerminal = computed(() => {
  if (selection.value.kind !== "terminal") return null;
  return terminals.value.find((t) => t.id === selection.value.id) ?? null;
});

const activeJob = computed(() => {
  if (selection.value.kind !== "job") return null;
  return jobs.value.find((j) => j.id === selection.value.id) ?? null;
});

const runningCount = computed(() => jobs.value.filter((j) => j.running).length);

function liveElapsedMs(job: BackgroundOutput): number {
  const base = Number(job.elapsedMs ?? 0);
  if (!job.running) return base;
  const delta = Math.max(0, nowTick.value - jobsSyncedAt.value);
  return base + delta;
}

function fmtMs(ms?: number): string {
  if (typeof ms !== "number" || !Number.isFinite(ms)) return "";
  if (ms < 1000) return `${Math.max(0, Math.floor(ms))}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  if (m < 60) return `${m}m ${rem}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function statusOf(job: BackgroundOutput): {
  text: string;
  type: "running" | "done" | "error" | "warn";
} {
  if (job.timedOut) return { text: "超时回收", type: "warn" };
  if (job.killed) return { text: "已终止", type: "error" };
  if (job.running) return { text: "运行中", type: "running" };
  if (job.finished) {
    return job.exitCode === 0
      ? { text: "已完成 · 0", type: "done" }
      : { text: `退出码 ${job.exitCode ?? "?"}`, type: "error" };
  }
  return { text: "未知", type: "warn" };
}

function cwdTail(cwd: string): string {
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return (parts.length ? parts[parts.length - 1] : cwd) || "workspace";
}

function setTerminalHost(termId: string, el: Element | ComponentPublicInstance | null) {
  if (el instanceof HTMLElement) {
    hostElements.set(termId, el);
  } else {
    hostElements.delete(termId);
  }
}

function ensureThemeObserver() {
  if (themeObserver) return;
  themeObserver = new MutationObserver(() => {
    const theme = getTerminalTheme();
    for (const handle of runtimes.values()) {
      handle.terminal.options.theme = theme;
    }
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });
}

async function ensureEventListener() {
  if (unlistenTerminal) return;
  unlistenTerminal = await listen<UserTerminalOutputEvent>(USER_TERMINAL_EVENT, (event) => {
    handleTerminalEvent(event.payload);
  });
}

async function pasteToTerminal(termId: string) {
  const term = terminals.value.find((t) => t.id === termId);
  if (!term || !term.sessionId) return;
  try {
    const text = await navigator.clipboard.readText();
    if (text) {
      await writeUserTerminal(normalizedConversationId(), text, term.id);
    }
  } catch (err) {
    console.warn("无法读取剪贴板:", err);
  }
}

function fitTerminal(termId: string) {
  if (!props.visible) return;
  if (selection.value.kind !== "terminal" || selection.value.id !== termId) return;
  const host = hostElements.get(termId);
  const handle = runtimes.get(termId);
  if (!host || !handle || host.clientWidth === 0 || host.clientHeight === 0) return;
  try {
    handle.fitAddon.fit();
    handle.terminal.scrollToBottom();
  } catch (err) {
    console.warn("Failed to fit terminal:", err);
  }
}

function scheduleResize(termId: string) {
  const handle = runtimes.get(termId);
  const term = terminals.value.find((t) => t.id === termId);
  if (!handle || !term) return;
  if (handle.resizeTimer !== null) {
    window.clearTimeout(handle.resizeTimer);
  }
  handle.resizeTimer = window.setTimeout(() => {
    handle.resizeTimer = null;
    fitTerminal(termId);
    if (term.sessionId) {
      void resizeUserTerminal(
        normalizedConversationId(),
        {
          rows: handle.terminal.rows,
          cols: handle.terminal.cols,
        },
        term.id,
      ).catch((err) => {
        console.warn("Failed to resize terminal:", err);
      });
    }
  }, 80);
}

async function createTerminalRuntime(termId: string) {
  if (runtimes.has(termId)) return;
  const host = hostElements.get(termId);
  if (!host) return;

  ensureThemeObserver();
  const terminal = new Terminal({
    cursorBlink: true,
    fontFamily: '"Cascadia Mono", "JetBrains Mono", Consolas, monospace',
    fontSize: 13,
    lineHeight: 1.2,
    scrollback: 8000,
    convertEol: false,
    theme: getTerminalTheme(),
  });

  terminal.attachCustomKeyEventHandler((event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c" && event.type === "keydown") {
      if (terminal.hasSelection()) {
        const text = terminal.getSelection();
        if (text) {
          void navigator.clipboard.writeText(text);
          terminal.clearSelection();
          return false;
        }
      }
    }
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "v" && event.type === "keydown") {
      void pasteToTerminal(termId);
      return false;
    }
    return true;
  });

  const fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(host);

  const inputDisposable = terminal.onData((data) => {
    const state = terminals.value.find((t) => t.id === termId);
    if (!state || !state.sessionId) return;
    void writeUserTerminal(normalizedConversationId(), data, termId).catch((err) => {
      state.error = String(err);
    });
  });

  const resizeObserver = new ResizeObserver(() => scheduleResize(termId));
  resizeObserver.observe(host);

  runtimes.set(termId, {
    terminal,
    fitAddon,
    resizeObserver,
    inputDisposable,
    resizeTimer: null,
  });

  await nextTick();
  scheduleResize(termId);
}

function disposeTerminalRuntime(termId: string) {
  const handle = runtimes.get(termId);
  if (!handle) return;
  if (handle.resizeTimer !== null) {
    window.clearTimeout(handle.resizeTimer);
  }
  handle.resizeObserver.disconnect();
  handle.inputDisposable.dispose();
  handle.terminal.dispose();
  runtimes.delete(termId);
}

function disposeAllRuntimes() {
  for (const id of [...runtimes.keys()]) {
    disposeTerminalRuntime(id);
  }
}

function handleTerminalEvent(payload: UserTerminalOutputEvent) {
  const sameConversation = (payload.conversationId?.trim() || null) === normalizedConversationId();
  if (!sameConversation) return;

  const targetId = payload.terminalId || "main";
  const term = terminals.value.find((t) => t.id === targetId || t.sessionId === payload.sessionId);
  const handle = term ? runtimes.get(term.id) : undefined;
  if (!term || !handle) return;

  if (payload.sessionId !== term.sessionId) {
    if (!term.sessionId) {
      term.sessionId = payload.sessionId;
    } else {
      return;
    }
  }

  if (payload.kind === "output") {
    handle.terminal.write(payload.data ?? "");
    handle.terminal.scrollToBottom();
    return;
  }

  if (payload.kind === "error") {
    handle.terminal.writeln(`\r\n[terminal error] ${payload.error ?? "unknown error"}`);
    handle.terminal.scrollToBottom();
    return;
  }

  if (payload.kind === "exit") {
    handle.terminal.writeln("\r\n[terminal exited]");
    handle.terminal.scrollToBottom();
    term.sessionId = null;
    term.alive = false;
  }
}

async function startOrAttachTerminal(termId: string) {
  if (!props.visible) return;
  const term = terminals.value.find((t) => t.id === termId);
  if (!term || term.isStarting) return;

  term.isStarting = true;
  term.error = "";

  try {
    await nextTick();
    if (!hostElements.get(termId)) return;
    await createTerminalRuntime(termId);
    await ensureEventListener();
    fitTerminal(termId);

    const handle = runtimes.get(termId);
    const info = await startUserTerminal(
      normalizedConversationId(),
      {
        rows: handle?.terminal.rows ?? 24,
        cols: handle?.terminal.cols ?? 80,
      },
      termId,
    );

    term.sessionId = info.sessionId;
    term.cwd = info.cwd;
    term.alive = true;
    if (info.shellName) {
      const sameShellCount = terminals.value.filter(
        (t) => t.id !== term.id && t.label.startsWith(info.shellName!),
      ).length;
      if (term.label === "终端" || term.label.startsWith("终端 ")) {
        term.label = sameShellCount > 0 ? `${info.shellName} (${sameShellCount + 1})` : info.shellName;
      }
    }

    await nextTick();
    scheduleResize(termId);
    handle?.terminal.scrollToBottom();
    if (selection.value.kind === "terminal" && selection.value.id === termId) {
      handle?.terminal.focus();
    }
  } catch (err) {
    term.error = String(err);
    emitToast({ variant: "error", source: "terminal-tab", message: "启动终端失败。" });
  } finally {
    term.isStarting = false;
  }
}

async function restartCurrentTerminal() {
  const term = activeTerminal.value;
  if (!term || term.isStarting) return;
  try {
    await stopUserTerminal(normalizedConversationId(), term.id);
    term.sessionId = null;
    term.alive = false;
    const handle = runtimes.get(term.id);
    handle?.terminal.clear();
    handle?.terminal.reset();
    await startOrAttachTerminal(term.id);
  } catch (err) {
    term.error = String(err);
    emitToast({ variant: "error", source: "terminal-tab", message: "重置终端失败。" });
  }
}

async function addNewTerminal() {
  terminalSeq += 1;
  const id = `term-${Date.now()}-${terminalSeq}`;
  const baseName = terminals.value.find((t) => t.label && t.label !== "终端")?.label.replace(/\s*\(\d+\)$/, "");
  const sameCount = baseName
    ? terminals.value.filter((t) => t.label.startsWith(baseName)).length
    : terminals.value.length;
  const label = baseName ? `${baseName} (${sameCount + 1})` : `终端 ${terminals.value.length + 1}`;

  terminals.value.push({
    id,
    label,
    sessionId: null,
    cwd: "workspace",
    isStarting: false,
    alive: false,
    error: "",
  });
  selection.value = { kind: "terminal", id };
  await startOrAttachTerminal(id);
}

async function closeTerminalTab(termId: string) {
  if (terminals.value.length <= 1) return;
  const idx = terminals.value.findIndex((t) => t.id === termId);
  if (idx === -1) return;

  try {
    await stopUserTerminal(normalizedConversationId(), termId);
  } catch (err) {
    console.warn("stopUserTerminal failed:", err);
  }
  disposeTerminalRuntime(termId);
  terminals.value.splice(idx, 1);

  if (selection.value.kind === "terminal" && selection.value.id === termId) {
    const nextTerm = terminals.value[Math.max(0, idx - 1)] ?? terminals.value[0];
    if (nextTerm) {
      await selectTerminal(nextTerm.id);
    }
  }
}

async function selectTerminal(termId: string) {
  selection.value = { kind: "terminal", id: termId };
  const term = terminals.value.find((t) => t.id === termId);
  if (!term) return;
  if (!term.sessionId || !runtimes.has(termId)) {
    await startOrAttachTerminal(termId);
  } else {
    await nextTick();
    scheduleResize(termId);
    runtimes.get(termId)?.terminal.focus();
  }
}

// 后台任务拉取与操作
async function fetchJobs(): Promise<BackgroundOutput[]> {
  const convId = normalizedConversationId();
  const seq = ++jobsRequestSeq;
  try {
    const list = await invoke<BackgroundOutput[]>("list_background_jobs", {
      conversationId: convId,
    });
    if (seq !== jobsRequestSeq || convId !== normalizedConversationId()) {
      return jobs.value;
    }
    jobs.value = list;
    jobsSyncedAt.value = Date.now();
    nowTick.value = jobsSyncedAt.value;

    // 若当前选中的后台任务已因 TTL 淘汰，自动回退到其他任务或交互终端
    if (selection.value.kind === "job" && !list.some((j) => j.id === selection.value.id)) {
      const nextJob = list.find((j) => j.running) ?? list[0];
      if (nextJob) {
        await selectJob(nextJob.id);
      } else {
        output.value = null;
        outputError.value = "";
        if (terminals.value[0]) {
          await selectTerminal(terminals.value[0].id);
        }
      }
    }
    return list;
  } catch (err) {
    console.error("list_background_jobs failed", err);
    return jobs.value;
  }
}

async function fetchOutput(targetId?: string) {
  const id = targetId ?? (selection.value.kind === "job" ? selection.value.id : null);
  if (!id) {
    output.value = null;
    outputError.value = "";
    loadingOutput.value = false;
    return;
  }

  const convId = normalizedConversationId();
  const seq = ++outputRequestSeq;
  try {
    const snap = await invoke<BackgroundOutput>("read_background_job_output", {
      conversationId: convId,
      id,
      tailLines: 400,
    });
    if (
      seq !== outputRequestSeq ||
      convId !== normalizedConversationId() ||
      selection.value.kind !== "job" ||
      selection.value.id !== id
    ) {
      return;
    }
    output.value = snap;
    outputError.value = "";
    loadingOutput.value = false;
    // 同步列表里对应的元信息与运行状态，保证列表与详情完全一致
    const idx = jobs.value.findIndex((j) => j.id === snap.id);
    if (idx !== -1) {
      jobs.value[idx] = { ...jobs.value[idx], ...snap };
      jobsSyncedAt.value = Date.now();
    }
  } catch (err) {
    if (
      seq !== outputRequestSeq ||
      convId !== normalizedConversationId() ||
      selection.value.kind !== "job" ||
      selection.value.id !== id
    ) {
      return;
    }
    loadingOutput.value = false;
    const message = String(err);
    outputError.value = message.includes("not found")
      ? "该后台任务已超过保留期并被回收。"
      : `读取输出失败：${message}`;
    if (message.includes("not found")) {
      await fetchJobs();
    }
  }
}

async function selectJob(id: string) {
  const isChangingJob = selection.value.kind !== "job" || selection.value.id !== id;
  selection.value = { kind: "job", id };
  outputError.value = "";
  if (isChangingJob || output.value?.id !== id) {
    output.value = null;
    loadingOutput.value = true;
  }
  await fetchOutput(id);
}

async function killJob(id: string) {
  if (killingId.value) return;
  const convId = normalizedConversationId();
  killingId.value = id;
  outputError.value = "";
  // 失效之前未完成的输出轮询，避免覆盖 kill 返回的新终态
  const seqGuard = ++outputRequestSeq;
  try {
    const killed = await invoke<BackgroundOutput>("kill_background_job", {
      conversationId: convId,
      id,
    });
    if (convId !== normalizedConversationId()) return;

    const idx = jobs.value.findIndex((j) => j.id === killed.id);
    if (idx !== -1) {
      jobs.value[idx] = killed;
    }
    jobsSyncedAt.value = Date.now();
    nowTick.value = jobsSyncedAt.value;

    if (selection.value.kind === "job" && selection.value.id === killed.id && seqGuard === outputRequestSeq) {
      output.value = killed;
      loadingOutput.value = false;
    }
    await fetchJobs();
  } catch (err) {
    const message = String(err);
    outputError.value = `终止任务失败：${message}`;
    emitToast({ variant: "error", source: "terminal-tab", message: `终止后台任务失败：${message}` });
  } finally {
    killingId.value = null;
  }
}

async function focusPreferredBackgroundJob() {
  const list = await fetchJobs();
  const preferred = list.find((j) => j.running) ?? list[0];
  if (preferred) {
    sidebarOpen.value = true;
    await selectJob(preferred.id);
  }
}

function startPolling() {
  stopPolling();
  void fetchJobs();
  if (selection.value.kind === "job") {
    void fetchOutput(selection.value.id);
  }
  pollTimer = setInterval(() => {
    void fetchJobs();
    if (selection.value.kind === "job") {
      void fetchOutput(selection.value.id);
    }
  }, 2000);
  clockTimer = setInterval(() => {
    nowTick.value = Date.now();
  }, 1000);
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  if (clockTimer) {
    clearInterval(clockTimer);
    clockTimer = null;
  }
}

function scrollOutputToBottom() {
  const el = outputEl.value;
  if (!el || !autoScroll.value) return;
  el.scrollTop = el.scrollHeight;
}

function handleOutputScroll() {
  const el = outputEl.value;
  if (!el) return;
  autoScroll.value = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
}

async function copyText(text: string, target: "cmd" | "cwd" | "output") {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    if (target === "cmd") {
      copiedCmd.value = true;
      setTimeout(() => { copiedCmd.value = false; }, 1400);
    } else if (target === "cwd") {
      copiedCwd.value = true;
      setTimeout(() => { copiedCwd.value = false; }, 1400);
    } else {
      copiedOutput.value = true;
      setTimeout(() => { copiedOutput.value = false; }, 1400);
    }
  } catch {
    // ignore clipboard failures
  }
}

function toggleSidebar() {
  sidebarOpen.value = !sidebarOpen.value;
  if (selection.value.kind === "terminal") {
    void nextTick().then(() => scheduleResize(selection.value.id));
  }
}

watch(
  () => [props.visible, props.conversationId, props.focusBgRequestKey] as const,
  async ([visible, _convId, focusKey]) => {
    const nextConv = normalizedConversationId();
    if (activeConversationKey.value !== nextConv) {
      disposeAllRuntimes();
      terminals.value = [
        {
          id: "main",
          label: "终端",
          sessionId: null,
          cwd: "workspace",
          isStarting: false,
          alive: false,
          error: "",
        },
      ];
      selection.value = { kind: "terminal", id: "main" };
      jobs.value = [];
      output.value = null;
      outputError.value = "";
      loadingOutput.value = false;
      killingId.value = null;
      activeConversationKey.value = nextConv;
    }

    if (!visible) {
      stopPolling();
      return;
    }

    startPolling();

    const shouldFocusBg = typeof focusKey === "number" && focusKey > lastHandledFocusKey;
    if (shouldFocusBg) {
      lastHandledFocusKey = focusKey;
      await focusPreferredBackgroundJob();
      return;
    }

    if (selection.value.kind === "terminal") {
      await startOrAttachTerminal(selection.value.id);
    } else {
      await fetchOutput(selection.value.id);
    }
  },
  { immediate: true },
);

watch(output, () => {
  void nextTick().then(scrollOutputToBottom);
});

onBeforeUnmount(() => {
  stopPolling();
  disposeAllRuntimes();
  themeObserver?.disconnect();
  if (unlistenTerminal) {
    void Promise.resolve(unlistenTerminal()).catch((err) => {
      console.warn("Failed to remove terminal listener:", err);
    });
  }
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-row bg-white text-[#111827] dark:bg-[#18181b] dark:text-[#f3f3f3]">
    <!-- 主视窗：根据右侧选中项切换「交互终端」或「后台任务输出」 -->
    <div class="flex min-w-0 flex-1 flex-col">
      <!-- 顶部上下文工具栏 -->
      <div class="flex h-10 shrink-0 items-center justify-between gap-2 border-b border-[#eef0f3] px-3 dark:border-[#27272a]">
        <!-- 交互终端工具栏左区 -->
        <template v-if="activeTerminal">
          <div class="flex min-w-0 items-center gap-2">
            <SquareTerminal class="h-3.5 w-3.5 shrink-0 text-[#64748b] dark:text-[#a1a1aa]" />
            <span class="truncate text-[12.5px] font-medium text-[#111827] dark:text-[#ececec]">
              {{ activeTerminal.label }}
            </span>
            <span class="truncate rounded bg-[#f1f5f9] px-1.5 py-0.5 font-mono text-[10.5px] text-[#64748b] dark:bg-[#27272a] dark:text-[#a1a1aa]">
              {{ cwdTail(activeTerminal.cwd) }}
            </span>
          </div>

          <div class="flex shrink-0 items-center gap-1">
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
              title="重启当前终端"
              :disabled="activeTerminal.isStarting"
              @click="restartCurrentTerminal"
            >
              <RotateCw class="h-3.5 w-3.5" />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
              title="滚动到底部"
              @click="runtimes.get(activeTerminal.id)?.terminal.scrollToBottom()"
            >
              <ArrowDownToLine class="h-3.5 w-3.5" />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
              title="适配尺寸"
              @click="scheduleResize(activeTerminal.id)"
            >
              <Maximize2 class="h-3.5 w-3.5" />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
              :class="sidebarOpen ? 'bg-[#f1f5f9] text-[#111827] dark:bg-[#27272a] dark:text-[#ececec]' : ''"
              :title="sidebarOpen ? '收起列表' : '展开列表'"
              @click="toggleSidebar"
            >
              <PanelRight class="h-3.5 w-3.5" />
            </Button>
          </div>
        </template>

        <!-- 后台任务工具栏左区 -->
        <template v-else-if="activeJob">
          <div class="flex min-w-0 items-center gap-2">
            <span
              class="h-2 w-2 shrink-0 rounded-full"
              :class="{
                'animate-pulse bg-emerald-500': statusOf(activeJob).type === 'running',
                'bg-[#94a3b8]': statusOf(activeJob).type === 'done',
                'bg-rose-500': statusOf(activeJob).type === 'error',
                'bg-amber-500': statusOf(activeJob).type === 'warn',
              }"
            />
            <span class="font-mono text-[12px] font-medium text-[#111827] dark:text-[#ececec]">
              #{{ activeJob.id }}
            </span>
            <span
              class="rounded px-1.5 py-0.5 text-[11px] font-medium"
              :class="{
                'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300': statusOf(activeJob).type === 'running',
                'bg-[#f1f5f9] text-[#475569] dark:bg-[#27272a] dark:text-[#a1a1aa]': statusOf(activeJob).type === 'done',
                'bg-rose-50 text-rose-700 dark:bg-rose-950/50 dark:text-rose-300': statusOf(activeJob).type === 'error',
                'bg-amber-50 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300': statusOf(activeJob).type === 'warn',
              }"
            >
              {{ statusOf(activeJob).text }}
            </span>
            <span class="truncate font-mono text-[11px] text-[#64748b] dark:text-[#a1a1aa]">
              pid {{ activeJob.pid }} · {{ fmtMs(liveElapsedMs(activeJob)) }}
            </span>
          </div>

          <div class="flex shrink-0 items-center gap-1.5">
            <button
              v-if="activeJob.running"
              type="button"
              class="inline-flex h-6.5 items-center gap-1 rounded-md border border-[#e7e9ee] bg-white px-2 text-[11px] font-medium text-rose-600 transition-colors hover:border-rose-200 hover:bg-rose-50 disabled:opacity-50 dark:border-[#343434] dark:bg-white/5 dark:text-rose-400 dark:hover:border-rose-900/60 dark:hover:bg-rose-950/40"
              :disabled="killingId === activeJob.id"
              @click="killJob(activeJob.id)"
            >
              <Square class="h-2.5 w-2.5 fill-current" />
              {{ killingId === activeJob.id ? "终止中…" : "终止" }}
            </button>
            <button
              type="button"
              class="h-6.5 rounded-md px-2 text-[11px] transition-colors"
              :class="autoScroll
                ? 'bg-[#f1f5f9] font-medium text-[#111827] dark:bg-[#27272a] dark:text-[#ececec]'
                : 'text-[#64748b] hover:bg-[#f1f5f9] hover:text-[#111827] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]'"
              @click="autoScroll = !autoScroll; scrollOutputToBottom()"
            >
              {{ autoScroll ? "贴底" : "浏览" }}
            </button>
            <button
              v-if="output && (output.stdout || output.stderr)"
              type="button"
              class="inline-flex h-6.5 items-center gap-1 rounded-md px-2 text-[11px] text-[#64748b] transition-colors hover:bg-[#f1f5f9] hover:text-[#111827] dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-[#ececec]"
              @click="copyText((output.stdout || '') + (output.stderr ? `\n[STDERR]\n${output.stderr}` : ''), 'output')"
            >
              <Check v-if="copiedOutput" class="h-3 w-3 text-emerald-500" />
              <Copy v-else class="h-3 w-3" />
              {{ copiedOutput ? "已复制" : "复制" }}
            </button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
              :class="sidebarOpen ? 'bg-[#f1f5f9] text-[#111827] dark:bg-[#27272a] dark:text-[#ececec]' : ''"
              :title="sidebarOpen ? '收起列表' : '展开列表'"
              @click="toggleSidebar"
            >
              <PanelRight class="h-3.5 w-3.5" />
            </Button>
          </div>
        </template>
      </div>

      <!-- 主内容区 -->
      <div class="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <!-- 1. 交互式终端集合：每个终端保持挂载以保留 scrollback -->
        <template v-for="term in terminals" :key="term.id">
          <div
            v-show="selection.kind === 'terminal' && selection.id === term.id"
            class="flex min-h-0 flex-1 flex-col"
          >
            <div
              v-if="term.error"
              class="border-b border-rose-100 bg-rose-50 px-3 py-1.5 text-xs text-rose-700 dark:border-rose-950/50 dark:bg-rose-950/30 dark:text-rose-200"
            >
              {{ term.error }}
            </div>
            <div
              :ref="(el) => setTerminalHost(term.id, el)"
              class="min-h-0 flex-1 overflow-hidden bg-white px-2 py-1 dark:bg-[#18181b]"
              @click="runtimes.get(term.id)?.terminal.focus()"
              @contextmenu.prevent="pasteToTerminal(term.id)"
            />
          </div>
        </template>

        <!-- 2. 选中后台任务的输出区 -->
        <div
          v-if="selection.kind === 'job' && activeJob"
          class="flex min-h-0 flex-1 flex-col bg-white dark:bg-[#18181b]"
        >
          <!-- 命令与工作目录条 -->
          <div class="flex shrink-0 flex-col gap-1 border-b border-[#eef0f3] bg-[#fafbfc] px-3 py-2 dark:border-[#27272a] dark:bg-[#141417]">
            <button
              type="button"
              class="group flex w-full items-center gap-1.5 text-left font-mono text-[11.5px] text-[#334155] transition-colors hover:text-[#0f172a] dark:text-[#cbd5e1] dark:hover:text-[#f8fafc]"
              :title="copiedCmd ? '已复制命令' : activeJob.command"
              @click="copyText(activeJob.command, 'cmd')"
            >
              <span class="shrink-0 select-none text-[#94a3b8]">$</span>
              <span class="min-w-0 flex-1 truncate">{{ activeJob.command }}</span>
              <Check v-if="copiedCmd" class="h-3 w-3 shrink-0 text-emerald-500" />
              <Copy v-else class="h-3 w-3 shrink-0 opacity-0 transition-opacity group-hover:opacity-60" />
            </button>
            <button
              v-if="activeJob.cwd"
              type="button"
              class="group flex w-full items-center gap-1.5 text-left font-mono text-[10.5px] text-[#64748b] transition-colors hover:text-[#334155] dark:text-[#71717a] dark:hover:text-[#a1a1aa]"
              :title="copiedCwd ? '已复制工作目录' : activeJob.cwd"
              @click="copyText(activeJob.cwd, 'cwd')"
            >
              <Folder class="h-3 w-3 shrink-0" />
              <span class="min-w-0 flex-1 truncate">{{ activeJob.cwd }}</span>
              <Check v-if="copiedCwd" class="h-3 w-3 shrink-0 text-emerald-500" />
              <Copy v-else class="h-3 w-3 shrink-0 opacity-0 transition-opacity group-hover:opacity-60" />
            </button>
          </div>

          <div
            v-if="outputError"
            class="border-b border-rose-100 bg-rose-50 px-3 py-1.5 text-xs text-rose-700 dark:border-rose-950/50 dark:bg-rose-950/30 dark:text-rose-200"
          >
            {{ outputError }}
          </div>

          <div
            v-if="output?.truncated"
            class="border-b border-[#eef0f3] bg-[#fafbfc] px-3 py-1 text-[10.5px] text-[#64748b] dark:border-[#27272a] dark:bg-[#141417] dark:text-[#71717a]"
          >
            输出已超出缓冲上限，当前展示前/后保留窗口的最近内容。
          </div>

          <div
            ref="outputEl"
            class="min-h-0 flex-1 overflow-y-auto px-3 py-2.5 custom-scrollbar"
            @scroll="handleOutputScroll"
          >
            <pre
              v-if="output && (output.stdout || output.stderr)"
              class="whitespace-pre-wrap break-words font-mono text-[12.5px] leading-[1.45] text-[#0f172a] dark:text-[#f4f4f5]"
              style='font-family: "Cascadia Mono", "JetBrains Mono", Consolas, monospace'
            >{{ output.stdout }}<span v-if="output.stderr" class="text-rose-600 dark:text-rose-400">{{ output.stderr }}</span></pre>

            <div
              v-else
              class="flex h-full items-center justify-center gap-2 text-[12px] text-[#64748b] dark:text-[#71717a]"
            >
              <template v-if="loadingOutput">
                <div class="h-3 w-3 animate-spin rounded-full border border-current border-r-transparent" />
                正在读取输出…
              </template>
              <template v-else-if="activeJob.running">
                <span class="h-2 w-2 animate-pulse rounded-full bg-emerald-500" />
                正在执行，暂未产生输出…
              </template>
              <template v-else>
                作业已结束，未捕获到输出内容
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 右侧分栏列表（参照终端 + 后台任务分组结构，可随时收起） -->
    <aside
      v-show="sidebarOpen"
      class="flex w-[212px] shrink-0 flex-col border-l border-[#eef0f3] bg-[#fafbfc] dark:border-[#27272a] dark:bg-[#141417]"
    >
      <div class="min-h-0 flex-1 overflow-y-auto p-2 custom-scrollbar">
        <!-- 分组 1：交互终端 -->
        <div class="mb-3">
          <div class="mb-1 flex items-center justify-between px-1.5">
            <span class="text-[11px] font-medium text-[#64748b] dark:text-[#a1a1aa]">
              终端
            </span>
            <button
              type="button"
              class="flex h-5 w-5 items-center justify-center rounded text-[#64748b] transition-colors hover:bg-[#e2e8f0]/70 hover:text-[#111827] dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-[#ececec]"
              title="新建终端"
              @click="addNewTerminal"
            >
              <Plus class="h-3.5 w-3.5" />
            </button>
          </div>

          <div class="flex flex-col gap-0.5">
            <div
              v-for="term in terminals"
              :key="term.id"
              role="button"
              tabindex="0"
              class="group flex h-7 cursor-pointer items-center gap-1.5 rounded-md px-2 text-[12px] transition-colors"
              :class="selection.kind === 'terminal' && selection.id === term.id
                ? 'bg-[#e9edf2] font-medium text-[#111827] dark:bg-[#27272a] dark:text-[#f4f4f5]'
                : 'text-[#475569] hover:bg-[#f1f5f9] hover:text-[#111827] dark:text-[#a1a1aa] dark:hover:bg-[#1f1f24] dark:hover:text-[#e4e4e7]'"
              @click="selectTerminal(term.id)"
              @keydown.enter="selectTerminal(term.id)"
            >
              <SquareTerminal class="h-3.5 w-3.5 shrink-0 opacity-75" />
              <span class="min-w-0 flex-1 truncate font-mono text-[11.5px]">{{ term.label }}</span>
              <button
                v-if="terminals.length > 1"
                type="button"
                class="flex h-4 w-4 shrink-0 items-center justify-center rounded text-[#64748b] opacity-0 transition-opacity hover:bg-black/10 hover:text-[#111827] group-hover:opacity-100 dark:text-[#a1a1aa] dark:hover:bg-white/10 dark:hover:text-[#f4f4f5]"
                title="关闭终端"
                @click.stop="closeTerminalTab(term.id)"
              >
                <X class="h-3 w-3" />
              </button>
            </div>
          </div>
        </div>

        <!-- 分组 2：后台任务 -->
        <div>
          <div class="mb-1 flex items-center justify-between px-1.5">
            <span class="text-[11px] font-medium text-[#64748b] dark:text-[#a1a1aa]">
              后台任务
            </span>
            <span
              v-if="jobs.length > 0"
              class="font-mono text-[10.5px] text-[#94a3b8] dark:text-[#71717a]"
            >
              {{ runningCount }}/{{ jobs.length }}
            </span>
          </div>

          <div v-if="jobs.length === 0" class="px-2 py-2 text-[11px] text-[#94a3b8] dark:text-[#71717a]">
            暂无后台任务
          </div>

          <div v-else class="flex flex-col gap-0.5">
            <div
              v-for="job in jobs"
              :key="job.id"
              role="button"
              tabindex="0"
              class="group flex h-7 cursor-pointer items-center gap-1.5 rounded-md px-2 text-[12px] transition-colors"
              :class="selection.kind === 'job' && selection.id === job.id
                ? 'bg-[#e9edf2] font-medium text-[#111827] dark:bg-[#27272a] dark:text-[#f4f4f5]'
                : 'text-[#475569] hover:bg-[#f1f5f9] hover:text-[#111827] dark:text-[#a1a1aa] dark:hover:bg-[#1f1f24] dark:hover:text-[#e4e4e7]'"
              :title="`#${job.id} (${statusOf(job).text}) · ${job.command}`"
              @click="selectJob(job.id)"
              @keydown.enter="selectJob(job.id)"
            >
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full"
                :class="{
                  'animate-pulse bg-emerald-500': statusOf(job).type === 'running',
                  'bg-[#94a3b8]': statusOf(job).type === 'done',
                  'bg-rose-500': statusOf(job).type === 'error',
                  'bg-amber-500': statusOf(job).type === 'warn',
                }"
              />
              <span class="min-w-0 flex-1 truncate font-mono text-[11.5px]">
                {{ job.command }}
              </span>
              <span
                class="shrink-0 font-mono text-[10px] text-[#94a3b8] dark:text-[#71717a]"
                :class="job.running ? 'group-hover:hidden' : ''"
              >
                {{ fmtMs(liveElapsedMs(job)) }}
              </span>
              <button
                v-if="job.running"
                type="button"
                class="hidden h-4 w-4 shrink-0 items-center justify-center rounded text-rose-600 transition-colors hover:bg-rose-100 group-hover:flex dark:text-rose-400 dark:hover:bg-rose-950/60"
                title="终止该任务"
                :disabled="killingId === job.id"
                @click.stop="killJob(job.id)"
              >
                <Square class="h-2.5 w-2.5 fill-current" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </aside>
  </div>
</template>

<style scoped>
.custom-scrollbar::-webkit-scrollbar {
  width: 5px;
  height: 5px;
}
.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}
.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: var(--color-border, #e5e5e5);
  border-radius: 10px;
}
</style>
