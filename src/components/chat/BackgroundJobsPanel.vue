<script setup lang="ts">
// 后台任务右侧面板：与执行计划（PlanPanel）同款抽屉外观。
// 只读：查看每个后台作业的命令、状态与最近输出，不提供终止操作（终止走 BashKill）。
// 打开期间轮询：列表 3s，选中作业的输出 2s。
import { computed, ref, watch, onBeforeUnmount } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { SquareTerminal, X } from "lucide-vue-next";
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

interface JobOutput extends BackgroundJob {
  // 输出经 read_background_job_output 拉取，结构同 BackgroundJob
}

const output = ref<JobOutput | null>(null);
const outputEl = ref<HTMLElement | null>(null);

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

// 输出更新后贴底，方便盯着 tail；用户往上翻了就不打扰。
function scrollToBottom() {
  const el = outputEl.value;
  if (!el) return;
  const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  if (nearBottom) el.scrollTop = el.scrollHeight;
}

function fmtMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${s % 60}s`;
}

/** 状态徽标的文案与配色。 */
function statusOf(job: BackgroundJob): { text: string; cls: string } {
  if (job.timedOut) return { text: "超时回收", cls: "bg-[#fef3c7] text-[#92400e] dark:bg-[#78350f]/40 dark:text-[#fcd34d]" };
  if (job.killed) return { text: "已终止", cls: "bg-[#fcebeb] text-[#a32d2d] dark:bg-[#7f1d1d]/40 dark:text-[#fca5a5]" };
  if (job.running) return { text: "运行中", cls: "bg-[#e6f1fb] text-[#185fa5] dark:bg-[#0c447c]/40 dark:text-[#93c5fd]" };
  if (job.finished) {
    return job.exitCode === 0
      ? { text: "已结束 · 0", cls: "bg-[#eaf3de] text-[#3b6d11] dark:bg-[#27500a]/40 dark:text-[#bef264]" }
      : { text: `退出码 ${job.exitCode ?? "?"}`, cls: "bg-[#fcebeb] text-[#a32d2d] dark:bg-[#7f1d1d]/40 dark:text-[#fca5a5]" };
  }
  return { text: "未知", cls: "bg-[#f1efe8] text-[#5f5e5a] dark:bg-white/10 dark:text-[#9ca3af]" };
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
    <aside v-show="open" class="relative z-20 h-full w-[440px] shrink-0 py-2 pl-1.5">
      <div
        class="flex h-full flex-col overflow-hidden rounded-2xl border border-[#e7e9ee] bg-white shadow-[0_8px_30px_rgba(15,23,42,0.08)] dark:border-[#343434] dark:bg-[#1e1e1e] dark:shadow-[0_8px_30px_rgba(0,0,0,0.4)]"
      >
        <div class="flex h-11 shrink-0 items-center justify-between border-b border-[#eef0f3] px-3 dark:border-[#2c2c2c]">
          <div class="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-[#111827] dark:text-[#ececec]">
            <SquareTerminal class="h-3.5 w-3.5 shrink-0 text-[#64748b] dark:text-[#9ca3af]" />
            后台任务
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-black/5 dark:hover:bg-white/5"
            title="关闭后台任务面板"
            @click="emit('close')"
          >
            <X class="h-4 w-4" />
          </Button>
        </div>

        <!-- 任务列表 -->
        <div class="max-h-[38%] shrink-0 overflow-y-auto border-b border-[#eef0f3] px-2 py-1.5 dark:border-[#2c2c2c]">
          <div v-if="jobs.length === 0" class="px-1.5 py-4 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
            当前会话没有后台作业。用 Bash(run_in_background: true) 启动后会出现在这里。
          </div>
          <button
            v-for="job in jobs"
            :key="job.id"
            type="button"
            class="flex w-full items-start gap-2 rounded-lg px-1.5 py-1.5 text-left transition-colors"
            :class="job.id === selectedId
              ? 'bg-[#f3f5f8] dark:bg-white/10'
              : 'hover:bg-[#f8fafc] dark:hover:bg-white/5'"
            @click="select(job.id)"
          >
            <span
              v-if="job.running"
              class="mt-1.5 h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-[#3b82f6]"
            />
            <span v-else class="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-[#cbd5e1] dark:bg-[#4b5563]" />
            <span class="min-w-0 flex-1">
              <span class="flex items-center gap-1.5">
                <span class="shrink-0 font-mono text-[11px] text-[#64748b] dark:text-[#9ca3af]">{{ job.id }}</span>
                <span class="shrink-0 rounded px-1 py-px text-[10px]" :class="statusOf(job).cls">{{ statusOf(job).text }}</span>
              </span>
              <span class="mt-0.5 block truncate font-mono text-[11px] text-[#374151] dark:text-[#d1d5db]" :title="job.command">{{ job.command }}</span>
            </span>
          </button>
        </div>

        <!-- 选中作业的详情与输出 -->
        <div class="flex min-h-0 flex-1 flex-col">
          <template v-if="selected">
            <div class="shrink-0 space-y-1 px-3 py-2 text-[11px] text-[#64748b] dark:text-[#9ca3af]">
              <div class="flex items-center gap-2">
                <span class="font-mono">{{ selected.id }}</span>
                <span>pid {{ selected.pid }}</span>
                <span>已运行 {{ fmtMs(selected.elapsedMs) }}</span>
                <span v-if="selected.running">剩余 {{ fmtMs(selected.remainingMs) }}</span>
              </div>
              <div v-if="selected.cwd" class="truncate" :title="selected.cwd">{{ selected.cwd }}</div>
            </div>
            <div
              ref="outputEl"
              class="min-h-0 flex-1 overflow-y-auto border-t border-[#eef0f3] px-3 py-2 dark:border-[#2c2c2c]"
            >
              <pre
                v-if="output && (output.stdout || output.stderr)"
                class="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-[#111827] dark:text-[#e5e5e5]"
              >{{ output.stdout }}<span v-if="output.stderr" class="text-[#b91c1c] dark:text-[#f87171]">{{ output.stderr }}</span></pre>
              <div v-else class="py-4 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
                {{ selected.running ? "还没有输出。" : "没有输出。" }}
              </div>
            </div>
          </template>
          <div v-else class="flex flex-1 items-center justify-center text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
            选择一个作业查看输出
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
