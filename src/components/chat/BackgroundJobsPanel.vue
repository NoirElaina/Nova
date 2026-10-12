<script setup lang="ts">
// 后台任务右侧面板：轻量现代化白净卡片风格（与智能体套件/子代理统一）。
// 只读：查看每个后台作业的命令、状态与最近输出。
// 打开期间轮询：列表 3s，选中作业的输出 2s。
import { computed, ref, watch, onBeforeUnmount } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import {
  SquareTerminal,
  X,
  Copy,
  Check,
  Folder,
  Terminal,
} from "lucide-vue-next";
import type { BackgroundJob } from "./BackgroundJobsChip.vue";

const props = defineProps<{
  open: boolean;
  conversationId: string | null;
}>();

const emit = defineEmits<{
  (e: "close"): void;
}>();

const jobs = ref<BackgroundJob[]>([]);
const selectedId = ref<string | null>(null);
let listTimer: ReturnType<typeof setInterval> | null = null;
let outputTimer: ReturnType<typeof setInterval> | null = null;

const selected = computed(() => jobs.value.find((j) => j.id === selectedId.value) ?? null);
const runningCount = computed(() => jobs.value.filter((j) => j.running).length);

interface JobOutput extends BackgroundJob {}

const output = ref<JobOutput | null>(null);
const outputEl = ref<HTMLElement | null>(null);
const autoScroll = ref(true);

async function fetchJobs() {
  try {
    jobs.value = await invoke<BackgroundJob[]>("list_background_jobs", {
      conversationId: props.conversationId,
    });
    // 默认选中第一个运行中的，否则第一个
    if (!jobs.value.some((j) => j.id === selectedId.value)) {
      selectedId.value = jobs.value.find((j) => j.running)?.id ?? jobs.value[0]?.id ?? null;
    }
  } catch (err) {
    console.error("list_background_jobs failed", err);
  }
}

async function fetchOutput() {
  if (!selectedId.value) {
    output.value = null;
    return;
  }
  try {
    output.value = await invoke<JobOutput>("read_background_job_output", {
      conversationId: props.conversationId,
      id: selectedId.value,
      tailLines: 400,
    });
  } catch (err) {
    console.error("read_background_job_output failed", err);
  }
}

function select(id: string) {
  selectedId.value = id;
  fetchOutput();
}

function startPolling() {
  stopPolling();
  void fetchJobs();
  void fetchOutput();
  listTimer = setInterval(fetchJobs, 3000);
  outputTimer = setInterval(fetchOutput, 2000);
}

function stopPolling() {
  if (listTimer) { clearInterval(listTimer); listTimer = null; }
  if (outputTimer) { clearInterval(outputTimer); outputTimer = null; }
}

function scrollToBottom() {
  const el = outputEl.value;
  if (!el || !autoScroll.value) return;
  el.scrollTop = el.scrollHeight;
}

function handleOutputScroll() {
  const el = outputEl.value;
  if (!el) return;
  const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  autoScroll.value = isNearBottom;
}

function fmtMs(ms?: number): string {
  if (typeof ms !== "number") return "";
  if (ms < 1000) return `${ms}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${s % 60}s`;
}

/** 状态徽标的文案与配色（轻量胶囊 Pill 规范）。 */
function statusOf(job: BackgroundJob): { text: string; type: "running" | "done" | "error" | "warn" } {
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

// 复制交互状态
const copiedOutput = ref(false);
async function copyOutputText() {
  if (!output.value) return;
  const text = (output.value.stdout || "") + (output.value.stderr ? `\n[STDERR]\n${output.value.stderr}` : "");
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    copiedOutput.value = true;
    setTimeout(() => { copiedOutput.value = false; }, 1500);
  } catch {
    // ignore
  }
}

const copiedCommand = ref<string | null>(null);
async function copyCommandText(cmd: string, id: string) {
  try {
    await navigator.clipboard.writeText(cmd);
    copiedCommand.value = id;
    setTimeout(() => { copiedCommand.value = null; }, 1500);
  } catch {
    // ignore
  }
}

const copiedCwd = ref(false);
async function copyCwdText(cwd?: string | null) {
  if (!cwd) return;
  try {
    await navigator.clipboard.writeText(cwd);
    copiedCwd.value = true;
    setTimeout(() => { copiedCwd.value = false; }, 1500);
  } catch {
    // ignore
  }
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      startPolling();
    } else {
      stopPolling();
    }
  },
);

watch(
  () => props.conversationId,
  () => {
    jobs.value = [];
    selectedId.value = null;
    output.value = null;
    if (props.open) startPolling();
  },
);

watch(output, () => {
  void Promise.resolve().then(scrollToBottom);
});

onBeforeUnmount(stopPolling);
</script>

<template>
  <Transition name="slide-right">
    <aside
      v-show="open"
      class="relative z-20 h-full w-[480px] shrink-0 py-2 pl-2"
      role="complementary"
      aria-label="后台任务面板"
    >
      <!-- 主容器：悬浮现代轻量卡片外壳 -->
      <div
        class="flex h-full flex-col overflow-hidden rounded-2xl border border-[#e5e7eb] bg-white shadow-[0_16px_40px_rgba(15,23,42,0.08)] dark:border-[#26282e] dark:bg-[#151619] dark:shadow-[0_20px_50px_rgba(0,0,0,0.5)]"
      >
        <!-- 顶部 Header -->
        <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#f1f5f9] px-4 dark:border-[#202227]">
          <div class="flex min-w-0 items-center gap-2.5">
            <div class="flex h-7 w-7 items-center justify-center rounded-lg bg-[#eff6ff] text-[#2563eb] dark:bg-[#1e293b] dark:text-[#60a5fa]">
              <SquareTerminal class="h-4 w-4" />
            </div>
            <span class="text-[14px] font-semibold tracking-tight text-[#0f172a] dark:text-[#f8fafc]">
              后台任务
            </span>
            <span
              v-if="runningCount > 0"
              class="inline-flex items-center gap-1.5 rounded-full border border-[#bbf7d0] bg-[#dcfce7] px-2 py-0.5 text-[11px] font-semibold text-[#15803d] dark:border-[#166534] dark:bg-emerald-950/40 dark:text-[#86efac]"
            >
              <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-[#22c55e]" />
              {{ runningCount }} 运行中
            </span>
            <span
              v-else-if="jobs.length > 0"
              class="rounded-full border border-[#e2e8f0] bg-[#f8fafc] px-2 py-0.5 text-[11px] font-medium text-[#64748b] dark:border-[#282a30] dark:bg-[#1c1d22] dark:text-[#94a3b8]"
            >
              {{ jobs.length }} 个任务
            </span>
          </div>

          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[#64748b] transition-colors hover:bg-black/5 hover:text-[#0f172a] dark:text-[#94a3b8] dark:hover:bg-white/5 dark:hover:text-[#ffffff]"
            title="关闭后台任务面板"
            @click="emit('close')"
          >
            <X class="h-4 w-4" />
          </Button>
        </div>

        <!-- 任务卡片列表区 -->
        <div class="max-h-[38%] shrink-0 overflow-y-auto border-b border-[#f1f5f9] p-3 custom-scrollbar dark:border-[#202227]">
          <div
            v-if="jobs.length === 0"
            class="flex flex-col items-center justify-center py-6 text-center"
          >
            <div class="flex h-10 w-10 items-center justify-center rounded-xl bg-[#f1f5f9] text-[#94a3b8] dark:bg-[#1e2025] dark:text-[#64748b] mb-2">
              <Terminal class="h-5 w-5" />
            </div>
            <p class="text-[13px] font-medium text-[#475569] dark:text-[#cbd5e1]">当前会话暂无后台作业</p>
            <p class="mt-0.5 max-w-[280px] text-[11.5px] text-[#94a3b8] dark:text-[#64748b]">
              使用 Bash(run_in_background: true) 启动命令后会实时出现在此处。
            </p>
          </div>

          <div v-else class="flex flex-col gap-2">
            <div
              v-for="job in jobs"
              :key="job.id"
              role="button"
              tabindex="0"
              class="group relative flex cursor-pointer flex-col gap-2 rounded-xl border p-2.5 transition-all"
              :class="job.id === selectedId
                ? 'border-[#3b82f6] bg-white shadow-sm ring-1 ring-[#3b82f6]/20 dark:border-[#60a5fa] dark:bg-[#1a1c22] dark:ring-[#60a5fa]/20'
                : 'border-[#e2e8f0] bg-[#fafbfc] hover:border-[#cbd5e1] hover:bg-[#f8fafc] dark:border-[#22242a] dark:bg-[#18191d] dark:hover:border-[#2d3038] dark:hover:bg-[#1e2025]'"
              @click="select(job.id)"
              @keydown.enter="select(job.id)"
            >
              <!-- 卡片首行：ID + 状态 Badge + 耗时/PID -->
              <div class="flex items-center justify-between">
                <div class="flex items-center gap-2">
                  <span class="rounded bg-black/5 px-1.5 py-0.5 font-mono text-[11px] font-semibold text-[#0f172a] dark:bg-white/10 dark:text-[#f8fafc]">
                    {{ job.id }}
                  </span>

                  <!-- 状态 Pill -->
                  <span
                    class="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10.5px] font-semibold"
                    :class="{
                      'border border-[#bbf7d0] bg-[#dcfce7] text-[#15803d] dark:border-[#166534] dark:bg-emerald-950/40 dark:text-[#86efac]': statusOf(job).type === 'running',
                      'border border-[#e2e8f0] bg-[#f1f5f9] text-[#475569] dark:border-[#282a30] dark:bg-[#1e2126] dark:text-[#94a3b8]': statusOf(job).type === 'done',
                      'border border-[#fecaca] bg-[#fee2e2] text-[#b91c1c] dark:border-[#991b1b] dark:bg-red-950/40 dark:text-[#fca5a5]': statusOf(job).type === 'error',
                      'border border-[#fde68a] bg-[#fef3c7] text-[#92400e] dark:border-[#854d0e] dark:bg-amber-950/40 dark:text-[#fcd34d]': statusOf(job).type === 'warn',
                    }"
                  >
                    <span
                      class="h-1.5 w-1.5 rounded-full"
                      :class="{
                        'animate-pulse bg-[#22c55e]': statusOf(job).type === 'running',
                        'bg-[#64748b] dark:bg-[#94a3b8]': statusOf(job).type === 'done',
                        'bg-[#ef4444]': statusOf(job).type === 'error',
                        'bg-[#f59e0b]': statusOf(job).type === 'warn',
                      }"
                    />
                    {{ statusOf(job).text }}
                  </span>
                </div>

                <div class="flex items-center gap-2 text-[11px] font-mono text-[#64748b] dark:text-[#94a3b8]">
                  <span>pid {{ job.pid }}</span>
                  <span v-if="fmtMs(job.elapsedMs)">· {{ fmtMs(job.elapsedMs) }}</span>
                </div>
              </div>

              <!-- 卡片第二行：命令内容小代码块 -->
              <div class="flex items-center justify-between gap-2 rounded-lg border border-[#e5e7eb] bg-white px-2 py-1 font-mono text-[11px] text-[#334155] dark:border-[#222429] dark:bg-[#121316] dark:text-[#cbd5e1]">
                <span class="truncate" :title="job.command">{{ job.command }}</span>
                <button
                  type="button"
                  class="shrink-0 p-0.5 text-[#94a3b8] opacity-60 transition-opacity hover:opacity-100 dark:text-[#64748b]"
                  :title="copiedCommand === job.id ? '已复制命令' : '复制命令'"
                  @click.stop="copyCommandText(job.command, job.id)"
                >
                  <Check v-if="copiedCommand === job.id" class="h-3 w-3 text-emerald-500" />
                  <Copy v-else class="h-3 w-3" />
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- 选中作业的详情与输出 -->
        <div class="flex min-h-0 flex-1 flex-col p-3">
          <template v-if="selected">
            <!-- 元数据小卡片 -->
            <div class="mb-2.5 flex flex-col gap-1.5 rounded-xl border border-[#e5e7eb] bg-[#fafbfc] p-2.5 text-[11.5px] dark:border-[#22242a] dark:bg-[#17181c]">
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex items-center gap-3 text-[#64748b] dark:text-[#94a3b8]">
                  <span class="font-mono font-semibold text-[#0f172a] dark:text-[#f8fafc]">{{ selected.id }}</span>
                  <span>PID <strong class="font-mono text-[#334155] dark:text-[#cbd5e1]">{{ selected.pid }}</strong></span>
                  <span>已运行 <strong class="font-mono text-[#334155] dark:text-[#cbd5e1]">{{ fmtMs(selected.elapsedMs) }}</strong></span>
                  <span v-if="selected.running">剩余 TTL <strong class="font-mono text-[#334155] dark:text-[#cbd5e1]">{{ fmtMs(selected.remainingMs) }}</strong></span>
                </div>
              </div>

              <!-- 工作目录 -->
              <div
                v-if="selected.cwd"
                class="flex items-center justify-between gap-1.5 rounded bg-black/5 px-2 py-1 font-mono text-[11px] text-[#475569] dark:bg-white/5 dark:text-[#94a3b8]"
              >
                <div class="flex min-w-0 items-center gap-1.5">
                  <Folder class="h-3 w-3 shrink-0 text-[#64748b] dark:text-[#94a3b8]" />
                  <span class="truncate" :title="selected.cwd">{{ selected.cwd }}</span>
                </div>
                <button
                  type="button"
                  class="shrink-0 p-0.5 text-[#94a3b8] opacity-70 transition-opacity hover:opacity-100 dark:text-[#64748b]"
                  :title="copiedCwd ? '已复制路径' : '复制工作目录'"
                  @click="copyCwdText(selected.cwd)"
                >
                  <Check v-if="copiedCwd" class="h-3 w-3 text-emerald-500" />
                  <Copy v-else class="h-3 w-3" />
                </button>
              </div>
            </div>

            <!-- 终端输出视窗卡片 -->
            <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-[#cbd5e1] bg-[#0e0f12] shadow-inner dark:border-[#23252c]">
              <!-- 终端 Header 栏 -->
              <div class="flex h-8 shrink-0 items-center justify-between border-b border-[#202227] bg-[#141519] px-3">
                <div class="flex items-center gap-1.5">
                  <span class="h-2 w-2 rounded-full bg-[#ef4444]" />
                  <span class="h-2 w-2 rounded-full bg-[#f59e0b]" />
                  <span class="h-2 w-2 rounded-full bg-[#10b981]" />
                  <span class="ml-1.5 font-mono text-[11px] font-medium text-[#94a3b8]">标准输出 (stdout / stderr)</span>
                </div>

                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    class="rounded px-1.5 py-0.5 text-[10.5px] font-medium transition-colors"
                    :class="autoScroll ? 'bg-white/10 text-white' : 'text-[#64748b] hover:text-[#94a3b8]'"
                    @click="autoScroll = !autoScroll; scrollToBottom()"
                  >
                    {{ autoScroll ? '贴底锁定' : '自由浏览' }}
                  </button>

                  <button
                    v-if="output && (output.stdout || output.stderr)"
                    type="button"
                    class="flex items-center gap-1 rounded bg-white/10 px-1.5 py-0.5 text-[10.5px] font-medium text-white transition-colors hover:bg-white/15"
                    @click="copyOutputText"
                  >
                    <Check v-if="copiedOutput" class="h-3 w-3 text-emerald-400" />
                    <Copy v-else class="h-3 w-3" />
                    <span>{{ copiedOutput ? '已复制' : '复制' }}</span>
                  </button>
                </div>
              </div>

              <!-- 终端输出主体 -->
              <div
                ref="outputEl"
                class="min-h-0 flex-1 overflow-y-auto p-3 custom-scrollbar"
                @scroll="handleOutputScroll"
              >
                <pre
                  v-if="output && (output.stdout || output.stderr)"
                  class="whitespace-pre-wrap break-words font-mono text-[11.5px] leading-relaxed text-[#e5e5e5]"
                >{{ output.stdout }}<span v-if="output.stderr" class="text-[#f87171]">{{ output.stderr }}</span></pre>

                <!-- 无输出等待状态 -->
                <div
                  v-else
                  class="flex h-full flex-col items-center justify-center py-8 text-center text-[#64748b]"
                >
                  <div v-if="selected.running" class="flex items-center gap-2 text-[12px]">
                    <div class="h-2.5 w-2.5 animate-spin rounded-full border border-solid border-current border-r-transparent text-emerald-400" />
                    <span>正在执行，等待输出产生…</span>
                  </div>
                  <div v-else class="text-[12px]">
                    作业已结束，未捕获到输出内容。
                  </div>
                </div>
              </div>
            </div>
          </template>

          <div
            v-else
            class="flex flex-1 flex-col items-center justify-center text-center text-[#94a3b8] dark:text-[#64748b]"
          >
            <SquareTerminal class="h-8 w-8 stroke-1 opacity-50 mb-1.5" />
            <p class="text-[12.5px]">请在上方选择一个作业查看详情与输出</p>
          </div>
        </div>
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
.slide-right-enter-active,
.slide-right-leave-active {
  transition:
    width 0.24s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.2s ease;
}

.slide-right-enter-from,
.slide-right-leave-to {
  width: 0 !important;
  opacity: 0;
}

.slide-right-enter-to,
.slide-right-leave-from {
  opacity: 1;
}
</style>
