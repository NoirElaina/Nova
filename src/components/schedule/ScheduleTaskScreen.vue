<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
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
const deletingIds = ref<Record<string, boolean>>({});
const tasks = ref<ScheduledTask[]>([]);

const cron = ref("*/15 * * * *");
const prompt = ref("");
const recurring = ref(true);
const durable = ref(false);

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const fieldClass =
  "border-[#d8dee8] bg-white text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]";
const labelClass = "text-[13px] text-[#374151] dark:text-[#d7d7d7]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const primaryButtonClass =
  "h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] focus-visible:ring-[#111827]/20 dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white";

const canCreate = computed(() => cron.value.trim().length > 0 && prompt.value.trim().length > 0);
const sortedTasks = computed(() => {
  return [...tasks.value].sort((a, b) => {
    const av = a.createdAt || "";
    const bv = b.createdAt || "";
    return bv.localeCompare(av);
  });
});

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
  } catch (err) {
    console.error("Failed to load scheduled tasks:", err);
  } finally {
    loading.value = false;
  }
}

async function handleCreateTask() {
  if (!canCreate.value || creating.value) {
    return;
  }

  creating.value = true;
  try {
    await createScheduledTask({
      cron: cron.value.trim(),
      prompt: prompt.value.trim(),
      recurring: recurring.value,
      durable: durable.value,
    });

    prompt.value = "";
    await loadTasks();
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

async function handleDeleteTask(id: string) {
  if (!id || deletingIds.value[id]) {
    return;
  }

  deletingIds.value = {
    ...deletingIds.value,
    [id]: true,
  };

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
    emitToast({
      variant: "success",
      source: "schedule",
      message: `已删除任务 ${id}。`,
    });
  } catch (err) {
    console.error("Failed to delete scheduled task:", err);
  } finally {
    const next = { ...deletingIds.value };
    delete next[id];
    deletingIds.value = next;
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
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">定时任务</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">管理 CronCreate / CronList / CronDelete 对应的任务列表。</p>
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

    <Card :class="panelClass">
      <CardHeader class="px-3 pb-0">
        <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">新建任务</CardTitle>
      </CardHeader>
      <CardContent class="space-y-3 px-3">
        <div class="space-y-1">
          <label :class="labelClass">Cron 表达式</label>
          <Input
            v-model="cron"
            :class="fieldClass"
            placeholder="例如: */15 * * * *"
          />
        </div>

        <div class="space-y-1">
          <label :class="labelClass">任务内容</label>
          <Textarea
            v-model="prompt"
            rows="3"
            :class="fieldClass"
            placeholder="到点要执行的提示词"
          />
        </div>

        <div class="flex flex-wrap items-center gap-4 text-sm text-[#475569] dark:text-[#d7d7d7]">
          <label class="inline-flex items-center gap-2 cursor-pointer">
            <input v-model="recurring" type="checkbox" class="rounded border-[#cbd5e1] accent-[#2563eb]" />
            <span>周期任务（recurring）</span>
          </label>
          <label class="inline-flex items-center gap-2 cursor-pointer">
            <input v-model="durable" type="checkbox" class="rounded border-[#cbd5e1] accent-[#2563eb]" />
            <span>跨重启持久化（durable）</span>
          </label>
        </div>

        <div class="pt-1">
          <Button
            size="sm"
            :class="primaryButtonClass"
            :disabled="!canCreate || creating"
            @click="handleCreateTask"
          >
            {{ creating ? '创建中...' : '创建定时任务' }}
          </Button>
        </div>
      </CardContent>
    </Card>

    <Card :class="panelClass">
      <CardHeader class="px-3 pb-0">
        <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">当前任务</CardTitle>
      </CardHeader>
      <CardContent class="space-y-2 px-3">
        <div v-if="loading" class="text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取任务...</div>
        <div v-else-if="sortedTasks.length === 0" class="rounded-lg border border-dashed border-[#d8dee8] px-3 py-6 text-center text-sm text-[#64748b] dark:border-[#3a3a3a] dark:text-[#a3a3a3]">暂无定时任务。</div>

        <div v-else class="space-y-2">
          <div
            v-for="task in sortedTasks"
            :key="task.id"
            class="flex items-start justify-between gap-3 rounded-lg border border-[#e5e7eb] bg-white px-3 py-2.5 transition-colors hover:bg-[#f8fafc] dark:border-[#333] dark:bg-[#242424] dark:hover:bg-[#2a2a2a]"
          >
            <div class="min-w-0 flex-1 space-y-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6]">{{ task.id }}</span>
                <span class="rounded-md bg-[#eef2f7] px-1.5 py-0.5 text-[11px] text-[#475569] dark:bg-[#303030] dark:text-[#cfcfcf]">{{ task.recurring ? '周期' : '一次性' }}</span>
                <span class="rounded-md bg-[#eef2f7] px-1.5 py-0.5 text-[11px] text-[#475569] dark:bg-[#303030] dark:text-[#cfcfcf]">{{ task.durable ? 'durable' : 'session' }}</span>
              </div>
              <div class="text-[12px] text-[#64748b] dark:text-[#a3a3a3]">cron: {{ task.cron }}</div>
              <div class="text-[12px] text-[#64748b] dark:text-[#a3a3a3]">conversation: {{ task.conversationId || '-' }}</div>
              <div class="break-words text-[13px] text-[#1f2937] dark:text-[#e5e7eb]">{{ task.prompt }}</div>
              <div class="text-[11px] text-[#94a3b8] dark:text-[#8b8b8b]">创建于 {{ formatDateTime(task.createdAt) }}</div>
            </div>

            <div class="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                class="h-7 px-2 text-[12px] text-[#2563eb] hover:bg-[#eff6ff] dark:text-[#93c5fd] dark:hover:bg-[#1e293b]"
                :disabled="!(task.conversationId && task.conversationId.trim())"
                @click="handleOpenTaskConversation(task)"
              >
                查看任务详细
              </Button>
              <Button
                variant="ghost"
                size="sm"
                class="h-7 px-2 text-[12px] text-[#dc2626] hover:bg-[#fef2f2] dark:text-[#fca5a5] dark:hover:bg-[#3a1f1f]"
                :disabled="!!deletingIds[task.id]"
                @click="handleDeleteTask(task.id)"
              >
                {{ deletingIds[task.id] ? '删除中...' : '删除' }}
              </Button>
            </div>
          </div>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
