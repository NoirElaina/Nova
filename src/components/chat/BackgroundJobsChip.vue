<script setup lang="ts">
// 输入框上方的后台任务小框：与任务进度（TodoChip）/计划（PlanChip）同款紧凑规格。
// 有后台作业才显示；展示 运行中/总数；点击打开右侧后台任务面板。
// 数据用轻量轮询：本地 invoke + tasklist 探活，3s 一次足够，不值得为此埋事件。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { SquareTerminal } from "lucide-vue-next";

export interface BackgroundJob {
  id: string;
  pid: number;
  command: string;
  cwd: string | null;
  running: boolean;
  finished: boolean;
  exitCode: number | null;
  killed: boolean;
  timedOut: boolean;
  stdout: string;
  stderr: string;
  truncated: boolean;
  elapsedMs: number;
  ttlMs: number;
  remainingMs: number;
}

const props = defineProps<{
  conversationId?: string | null;
}>();

const emit = defineEmits<{
  (e: "open"): void;
}>();

const jobs = ref<BackgroundJob[]>([]);
let pollTimer: ReturnType<typeof setInterval> | null = null;

const total = computed(() => jobs.value.length);
const running = computed(() => jobs.value.filter((j) => j.running).length);

async function fetchJobs() {
  if (!props.conversationId) {
    jobs.value = [];
    return;
  }
  try {
    jobs.value = await invoke<BackgroundJob[]>("list_background_jobs", {
      conversationId: props.conversationId,
    });
  } catch (err) {
    console.error("list_background_jobs failed", err);
  }
}

function startPolling() {
  stopPolling();
  pollTimer = setInterval(fetchJobs, 3000);
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

watch(
  () => props.conversationId,
  () => {
    jobs.value = [];
    fetchJobs();
  },
);

onMounted(() => {
  fetchJobs();
  startPolling();
});

onBeforeUnmount(stopPolling);
</script>

<template>
  <button
    v-if="total > 0"
    type="button"
    class="inline-flex h-7 max-w-full items-center gap-1.5 rounded-lg border border-[#e7e9ee] bg-[#fafbfc] px-2 text-left transition-colors hover:bg-[#f3f5f8] dark:border-[#343434] dark:bg-white/5 dark:hover:bg-white/10"
    :title="`后台任务 ${running}/${total} 运行中`"
    @click="emit('open')"
  >
    <SquareTerminal class="h-3.5 w-3.5 shrink-0 text-[#64748b] dark:text-[#9ca3af]" />
    <span class="shrink-0 text-[11px] text-[#8a94a3] dark:text-[#858585]">后台任务</span>
    <span
      class="shrink-0 text-[12px] font-medium"
      :class="running > 0
        ? 'text-[#111827] dark:text-[#ececec]'
        : 'text-[#64748b] dark:text-[#94a3b8]'"
    >
      {{ running }}/{{ total }}
    </span>
    <span
      v-if="running > 0"
      class="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-[#3b82f6]"
    />
  </button>
</template>
