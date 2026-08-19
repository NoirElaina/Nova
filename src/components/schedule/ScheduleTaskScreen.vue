<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { emitToast } from "../../lib/toast";
import type { ScheduledTask } from "../../lib/chat-types";
import {
  createScheduledTask,
  deleteScheduledTask,
  listScheduledTasks,
} from "../../features/chat/services/chat-api";

type MainView = "chat" | "hooks" | "agent" | "schedule";

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
  (e: "open-task-conversation", conversationId: string): void;
}>();

const loading = ref(false);
const creating = ref(false);
const deleting = ref(false);
const tasks = ref<ScheduledTask[]>([]);

/** 页面两级视图：grid = 任务卡片列表；detail = 单个任务详情；create = 新建任务。 */
const view = ref<"grid" | "detail" | "create">("grid");
const selectedTaskId = ref("");

// ---------------- 新建表单：日历式选择 → 编译为 cron ----------------

type RepeatMode = "once" | "daily" | "weekly" | "monthly" | "custom";
const repeatMode = ref<RepeatMode>("daily");
const repeatModeLabels: Record<RepeatMode, string> = {
  once: "单次",
  daily: "每天",
  weekly: "每周",
  monthly: "每月",
  custom: "自定义（Cron）",
};
/** 周几按钮组，UI 按周一开头排列；value 为 cron day-of-week 数字（0=周日）。 */
const WEEKDAYS: { value: number; label: string }[] = [
  { value: 1, label: "一" },
  { value: 2, label: "二" },
  { value: 3, label: "三" },
  { value: 4, label: "四" },
  { value: 5, label: "五" },
  { value: 6, label: "六" },
  { value: 0, label: "日" },
];
const MONTH_DAYS = Array.from({ length: 31 }, (_, i) => String(i + 1));

const prompt = ref("");
const timeValue = ref("09:00");
const dateValue = ref(todayStr());
const weekdaySelection = ref<Set<number>>(new Set([1]));
const monthDay = ref("1");
const customCron = ref("*/15 * * * *");
const durable = ref(false);

function todayStr(): string {
  const now = new Date();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${m}-${d}`;
}

/** 当前选择编译出的 cron 表达式（无效输入返回空串）。 */
const compiledCron = computed<string>(() => {
  const [h, m] = timeValue.value.split(":");
  const hour = Number(h);
  const minute = Number(m);
  if (!Number.isInteger(hour) || !Number.isInteger(minute)) return "";

  switch (repeatMode.value) {
    case "once": {
      const [y, mo, d] = dateValue.value.split("-").map(Number);
      if (!Number.isInteger(y) || !Number.isInteger(mo) || !Number.isInteger(d)) return "";
      return `${minute} ${hour} ${d} ${mo} *`;
    }
    case "daily":
      return `${minute} ${hour} * * *`;
    case "weekly": {
      if (weekdaySelection.value.size === 0) return "";
      const days = [...weekdaySelection.value].sort((a, b) => a - b).join(",");
      return `${minute} ${hour} * * ${days}`;
    }
    case "monthly":
      return `${minute} ${hour} ${Number(monthDay.value)} * *`;
    case "custom":
      return customCron.value.trim();
  }
});

/** 单次任务需校验时刻在未来（调度器只按 cron 分钟匹配，过期即永不触发）。 */
const onceInPast = computed(() => {
  if (repeatMode.value !== "once") return false;
  const ts = new Date(`${dateValue.value}T${timeValue.value}`);
  if (Number.isNaN(ts.getTime())) return true;
  return ts.getTime() <= Date.now();
});

const canCreate = computed(() => prompt.value.trim().length > 0 && compiledCron.value.length > 0 && !onceInPast.value);

const cronPreviewText = computed(() => {
  const cron = compiledCron.value;
  if (!cron) return "请完善时间设置";
  return `${describeCron(cron, repeatMode.value !== "once")}（cron: ${cron}）`;
});

// ---------------- cron → 可读描述 ----------------

const WEEKDAY_NAMES: Record<string, string> = {
  "0": "周日",
  "1": "周一",
  "2": "周二",
  "3": "周三",
  "4": "周四",
  "5": "周五",
  "6": "周六",
  "7": "周日",
};

function describeCron(cron: string, recurring: boolean): string {
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 5) return cron;
  const [min, hour, dom, month, dow] = parts;

  const restAll = dom === "*" && month === "*" && dow === "*";
  if (min === "*" && hour === "*" && restAll) return "每分钟";

  const everyMin = /^\*\/(\d+)$/.exec(min);
  if (everyMin && hour === "*" && restAll) return `每 ${everyMin[1]} 分钟`;

  const everyHour = /^\*\/(\d+)$/.exec(hour);
  if (everyHour && restAll) {
    const mm = min === "*" ? "00" : min.padStart(2, "0");
    return `每 ${everyHour[1]} 小时的 ${mm} 分`;
  }

  const isPlain = (v: string) => /^\d{1,2}$/.test(v);
  if (isPlain(hour) && isPlain(min)) {
    const time = `${hour.padStart(2, "0")}:${min.padStart(2, "0")}`;
    if (restAll) return `每天 ${time}`;
    if (isPlain(dom) && isPlain(month) && dow === "*" && !recurring) {
      return `${month.padStart(2, "0")}月${dom.padStart(2, "0")}日 ${time}（单次）`;
    }
    if (dom === "*" && month === "*" && dow !== "*") {
      if (dow === "1-5") return `工作日（周一至周五）${time}`;
      if (dow === "0,6" || dow === "6,0") return `周末（周六、周日）${time}`;
      const days = dow.split(",").filter(Boolean);
      if (days.length > 0 && days.every((d) => d in WEEKDAY_NAMES)) {
        const labels = days.map((d) => WEEKDAY_NAMES[d]);
        return `每${labels.join("、")} ${time}`;
      }
    }
    if (isPlain(dom) && month === "*" && dow === "*") {
      return `每月 ${Number(dom)} 号 ${time}`;
    }
  }

  return cron;
}

// ---------------- 样式常量 ----------------

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const fieldClass =
  "border-[#d8dee8] bg-white text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]";
/** 原生 date/time 控件：靠 color-scheme 适配暗色主题。 */
const nativeFieldClass =
  "h-9 w-full rounded-md border border-[#d8dee8] bg-white px-3 text-[14px] text-[#111827] outline-none transition-colors [color-scheme:light] focus:border-[#2563eb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#ededed] dark:focus:border-[#60a5fa] dark:[color-scheme:dark]";
const labelClass = "text-[13px] text-[#374151] dark:text-[#d7d7d7]";
const valueClass =
  "rounded-md border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2 text-[13px] text-[#1f2937] dark:border-[#333] dark:bg-[#262626] dark:text-[#e5e7eb]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const primaryButtonClass =
  "h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] focus-visible:ring-[#111827]/20 dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white";
const destructiveButtonClass =
  "h-8 border border-[#fecaca] bg-white px-3 text-[13px] text-[#dc2626] shadow-none hover:bg-[#fef2f2] dark:border-[#513030] dark:bg-[#242424] dark:text-[#fca5a5] dark:hover:bg-[#3a1f1f]";

// ---------------- 数据加载与操作 ----------------

const sortedTasks = computed(() => {
  return [...tasks.value].sort((a, b) => {
    const av = a.createdAt || "";
    const bv = b.createdAt || "";
    return bv.localeCompare(av);
  });
});
const selectedTask = computed(
  () => tasks.value.find((t) => t.id === selectedTaskId.value) ?? null,
);

function formatDateTime(iso?: string): string {
  if (!iso) return "-";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

async function loadTasks() {
  loading.value = true;
  try {
    tasks.value = await listScheduledTasks();
    // 详情打开中：选中任务已不存在（被外部删除）则退回卡片列表。
    if (view.value === "detail" && !tasks.value.some((t) => t.id === selectedTaskId.value)) {
      selectedTaskId.value = "";
      view.value = "grid";
    }
  } catch (err) {
    console.error("Failed to load scheduled tasks:", err);
  } finally {
    loading.value = false;
  }
}

function openTaskDetail(task: ScheduledTask) {
  selectedTaskId.value = task.id;
  view.value = "detail";
}

function openCreatePage() {
  repeatMode.value = "daily";
  prompt.value = "";
  timeValue.value = "09:00";
  dateValue.value = todayStr();
  weekdaySelection.value = new Set([1]);
  monthDay.value = "1";
  customCron.value = "*/15 * * * *";
  durable.value = false;
  view.value = "create";
}

function backToGrid() {
  view.value = "grid";
}

function toggleWeekday(value: number) {
  const next = new Set(weekdaySelection.value);
  if (next.has(value)) {
    next.delete(value);
  } else {
    next.add(value);
  }
  weekdaySelection.value = next;
}

async function handleCreateTask() {
  const cron = compiledCron.value;
  if (!canCreate.value || creating.value) {
    return;
  }

  creating.value = true;
  try {
    await createScheduledTask({
      cron,
      prompt: prompt.value.trim(),
      recurring: repeatMode.value !== "once",
      durable: durable.value,
    });

    await loadTasks();
    view.value = "grid";
    emitToast({
      variant: "success",
      source: "schedule",
      message: "定时任务已创建。",
    });
  } catch (err) {
    console.error("Failed to create scheduled task:", err);
  } finally {
    creating.value = false;
  }
}

async function handleDeleteTask() {
  const id = selectedTaskId.value;
  if (!id || deleting.value) {
    return;
  }

  deleting.value = true;
  try {
    const removed = await deleteScheduledTask(id);
    if (!removed) {
      emitToast({
        variant: "error",
        source: "schedule",
        message: `任务 ${id} 不存在或已删除。`,
      });
      return;
    }

    await loadTasks();
    selectedTaskId.value = "";
    view.value = "grid";
    emitToast({
      variant: "success",
      source: "schedule",
      message: `已删除任务 ${id}。`,
    });
  } catch (err) {
    console.error("Failed to delete scheduled task:", err);
  } finally {
    deleting.value = false;
  }
}

function handleOpenTaskConversation(task: ScheduledTask) {
  const conversationId = (task.conversationId ?? "").trim();
  if (!conversationId) {
    emitToast({
      variant: "error",
      source: "schedule",
      message: `任务 ${task.id} 缺少绑定会话，无法打开。`,
    });
    return;
  }

  emit("open-task-conversation", conversationId);
}

onMounted(() => {
  loadTasks();
});
</script>

<template>
  <div :class="pageClass">
    <header v-if="view === 'grid'" class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">定时任务</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">管理 CronCreate / CronList / CronDelete 对应的任务列表。点击卡片查看任务详情。</p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          :class="headerButtonClass"
          @click="emit('change-main-view', 'chat')"
        >
          返回聊天
        </Button>
        <Button
          variant="ghost"
          size="sm"
          :class="headerButtonClass"
          :disabled="loading || creating"
          @click="loadTasks"
        >
          刷新
        </Button>
      </div>
    </header>

    <Card v-if="loading" :class="panelClass">
      <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取任务...</CardContent>
    </Card>

    <!-- 卡片网格视图：每个任务一张卡，点击进入详情页 -->
    <div v-else-if="view === 'grid'" class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
      <button
        v-for="task in sortedTasks"
        :key="task.id"
        type="button"
        class="flex min-h-[120px] cursor-pointer flex-col rounded-xl border border-[#e5e7eb] bg-white p-4 text-left transition-all hover:border-[#2563eb]/60 hover:shadow-[0_6px_20px_rgba(37,99,235,0.10)] dark:border-[#333] dark:bg-[#242424] dark:hover:border-[#60a5fa]/50 dark:hover:shadow-none"
        :title="`查看任务 ${task.id}`"
        @click="openTaskDetail(task)"
      >
        <div class="flex items-center justify-between gap-2">
          <span class="truncate text-[13.5px] font-semibold text-[#111827] dark:text-[#f3f4f6]">{{ describeCron(task.cron, task.recurring) }}</span>
          <span
            class="shrink-0 rounded-md bg-[#eef2f7] px-1.5 py-0.5 text-[11px] text-[#475569] dark:bg-[#303030] dark:text-[#cfcfcf]"
          >{{ task.recurring ? '周期' : '一次性' }}</span>
        </div>
        <p class="mt-1.5 line-clamp-2 flex-1 break-words text-[12.5px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">
          {{ task.prompt }}
        </p>
        <div class="mt-2 flex items-center gap-2 text-[11px] text-[#98a2b3] dark:text-[#9d9589]">
          <span class="truncate font-mono">{{ task.cron }}</span>
          <span>·</span>
          <span class="shrink-0">{{ formatDateTime(task.createdAt) }}</span>
        </div>
      </button>

      <!-- 新建卡片 -->
      <button
        type="button"
        class="flex min-h-[120px] cursor-pointer flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-[#c7d4e8] bg-white/40 p-4 text-[#64748b] transition-colors hover:border-[#2563eb] hover:bg-[#f8fafc] hover:text-[#2563eb] dark:border-[#3f3f3f] dark:bg-transparent dark:text-[#a3a3a3] dark:hover:border-[#60a5fa] dark:hover:bg-[#2a2a2a] dark:hover:text-[#93c5fd]"
        @click="openCreatePage"
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        <span class="text-[13px] font-medium">新建任务</span>
      </button>
    </div>

    <!-- 任务详情视图 -->
    <Card v-else-if="view === 'detail' && selectedTask" :class="panelClass">
      <CardContent class="space-y-4 px-4 pb-4">
        <!-- 顶部：返回 + 任务信息 + 操作 -->
        <div class="flex flex-wrap items-center justify-between gap-3 border-b border-[#e5e7eb] pb-3 dark:border-[#333]">
          <div class="flex min-w-0 items-center gap-2.5">
            <Button variant="outline" size="sm" :class="headerButtonClass" @click="backToGrid">
              ← 返回
            </Button>
            <div class="min-w-0">
              <div class="truncate font-mono text-[14px] font-semibold text-[#111827] dark:text-[#f3f4f6]">
                {{ selectedTask.id }}
              </div>
              <div class="mt-0.5 text-[11.5px] text-[#98a2b3] dark:text-[#9d9589]">
                创建于 {{ formatDateTime(selectedTask.createdAt) }}
              </div>
            </div>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              :class="headerButtonClass"
              :disabled="!(selectedTask.conversationId && selectedTask.conversationId.trim())"
              @click="handleOpenTaskConversation(selectedTask)"
            >
              查看任务详细
            </Button>
            <Button
              variant="outline"
              size="sm"
              :class="destructiveButtonClass"
              :disabled="deleting"
              @click="handleDeleteTask"
            >
              {{ deleting ? '删除中...' : '删除' }}
            </Button>
          </div>
        </div>

        <!-- 基本信息 -->
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div class="space-y-1">
            <label :class="labelClass">执行计划</label>
            <div :class="valueClass">
              {{ describeCron(selectedTask.cron, selectedTask.recurring) }}
              <span class="ml-1.5 font-mono text-[11px] text-[#94a3b8] dark:text-[#8b8b8b]">{{ selectedTask.cron }}</span>
            </div>
          </div>
          <div class="space-y-1">
            <label :class="labelClass">任务类型</label>
            <div :class="valueClass">
              {{ selectedTask.recurring ? '周期任务（recurring）' : '一次性任务' }} ·
              {{ selectedTask.durable ? '跨重启持久化（durable）' : '会话级（session）' }}
            </div>
          </div>
        </div>

        <div class="space-y-1">
          <label :class="labelClass">绑定会话</label>
          <div :class="valueClass" class="break-all font-mono">{{ selectedTask.conversationId || '-' }}</div>
        </div>

        <div class="space-y-1">
          <label :class="labelClass">任务内容</label>
          <div :class="valueClass" class="whitespace-pre-wrap break-words">{{ selectedTask.prompt }}</div>
        </div>
      </CardContent>
    </Card>

    <!-- 新建任务视图 -->
    <Card v-else-if="view === 'create'" :class="panelClass">
      <CardContent class="space-y-4 px-4 pb-4">
        <!-- 顶部：返回 + 创建 -->
        <div class="flex flex-wrap items-center justify-between gap-3 border-b border-[#e5e7eb] pb-3 dark:border-[#333]">
          <div class="flex min-w-0 items-center gap-2.5">
            <Button variant="outline" size="sm" :class="headerButtonClass" @click="backToGrid">
              ← 返回
            </Button>
            <div class="text-[15px] font-semibold text-[#111827] dark:text-[#f3f4f6]">新建任务</div>
          </div>
          <Button
            size="sm"
            :class="primaryButtonClass"
            :disabled="!canCreate || creating"
            @click="handleCreateTask"
          >
            {{ creating ? '创建中...' : '创建定时任务' }}
          </Button>
        </div>

        <!-- 重复方式 -->
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div class="space-y-1">
            <label :class="labelClass">重复方式</label>
            <Select v-model="repeatMode">
              <SelectTrigger class="w-full text-[14px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem v-for="(label, mode) in repeatModeLabels" :key="mode" :value="mode">
                  {{ label }}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          <!-- 执行时间（自定义 cron 模式不适用） -->
          <div v-if="repeatMode !== 'custom'" class="space-y-1">
            <label :class="labelClass">执行时间</label>
            <input
              v-model="timeValue"
              type="time"
              :class="nativeFieldClass"
            />
          </div>

          <!-- 单次：选日期 -->
          <div v-if="repeatMode === 'once'" class="space-y-1">
            <label :class="labelClass">执行日期</label>
            <input
              v-model="dateValue"
              type="date"
              :class="nativeFieldClass"
            />
            <p v-if="onceInPast" class="text-[11.5px] text-destructive">
              所选时刻已过去，单次任务不会触发，请选择未来时间。
            </p>
          </div>

          <!-- 每月：每月几号 -->
          <div v-if="repeatMode === 'monthly'" class="space-y-1">
            <label :class="labelClass">每月几号</label>
            <Select v-model="monthDay">
              <SelectTrigger class="w-full text-[14px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent class="max-h-[240px]">
                <SelectItem v-for="d in MONTH_DAYS" :key="d" :value="d">{{ d }} 号</SelectItem>
              </SelectContent>
            </Select>
            <p class="text-[11px] text-[#7b8494] dark:text-[#9ca3af]">所选月份没有该日期时（如 31 号）当月跳过。</p>
          </div>
        </div>

        <!-- 每周：周几多选 -->
        <div v-if="repeatMode === 'weekly'" class="space-y-1.5">
          <label :class="labelClass">重复于（至少选一天）</label>
          <div class="flex gap-1.5">
            <button
              v-for="day in WEEKDAYS"
              :key="day.value"
              type="button"
              class="h-9 flex-1 cursor-pointer rounded-md border text-[13px] font-medium transition-colors"
              :class="weekdaySelection.has(day.value)
                ? 'border-[#2563eb] bg-[#2563eb] text-white shadow-none'
                : 'border-[#d8dee8] bg-white text-[#475569] hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]'"
              :aria-pressed="weekdaySelection.has(day.value)"
              @click="toggleWeekday(day.value)"
            >
              {{ day.label }}
            </button>
          </div>
          <p v-if="weekdaySelection.size === 0" class="text-[11.5px] text-destructive">请至少选择一天。</p>
        </div>

        <!-- 自定义 cron -->
        <div v-if="repeatMode === 'custom'" class="space-y-1">
          <label :class="labelClass">Cron 表达式</label>
          <Input
            v-model="customCron"
            :class="fieldClass"
            class="font-mono"
            placeholder="*/15 * * * *"
          />
          <p class="text-[11px] text-[#7b8494] dark:text-[#9ca3af]">标准 5 字段：分 时 日 月 周。</p>
        </div>

        <!-- 任务内容 -->
        <div class="space-y-1">
          <label :class="labelClass">任务内容</label>
          <Textarea
            v-model="prompt"
            rows="5"
            :class="fieldClass"
            placeholder="到点要执行的提示词"
          />
        </div>

        <div class="flex flex-wrap items-center gap-4 text-sm text-[#475569] dark:text-[#d7d7d7]">
          <label class="inline-flex items-center gap-2 cursor-pointer">
            <input v-model="durable" type="checkbox" class="rounded border-[#cbd5e1] accent-[#2563eb]" />
            <span>跨重启持久化（durable）</span>
          </label>
        </div>

        <!-- 生成结果预览 -->
        <div class="rounded-md border border-dashed border-[#cbd5e1] bg-[#f8fafc] px-3 py-2 text-[12px] text-[#475569] dark:border-[#3f3f46] dark:bg-[#262626] dark:text-[#9ca3af]">
          {{ cronPreviewText }}
        </div>
      </CardContent>
    </Card>
  </div>
</template>
