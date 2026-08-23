<script setup lang="ts">
// 输入框上方的任务进度小框：与计划小框（PlanChip）同款紧凑规格。
// 有任务清单才显示；点击向上弹出任务列表面板，点击外部关闭。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ListChecks, ListMinus, ListTodo } from "lucide-vue-next";

interface TodoEntry {
  id: string;
  content: string;
  status: string;
  priority: string;
}

const props = defineProps<{
  conversationId?: string | null;
}>();

const rootRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);
const todos = ref<TodoEntry[]>([]);
const loading = ref(false);
let unlistenFn: UnlistenFn | null = null;

const total = computed(() => todos.value.length);
const done = computed(() => todos.value.filter((t) => t.status === "completed").length);

async function fetchTodos() {
  if (!props.conversationId) {
    todos.value = [];
    return;
  }
  loading.value = true;
  try {
    const result = await invoke<TodoEntry[]>("list_todos", {
      conversationId: props.conversationId,
    });
    todos.value = result;
  } catch (err) {
    console.error("list_todos failed", err);
    todos.value = [];
  } finally {
    loading.value = false;
  }
}

const togglePanel = () => {
  isOpen.value = !isOpen.value;
  if (isOpen.value) {
    fetchTodos();
  }
};

/** 每条任务左侧的状态图标：待办/进行中/已完成。 */
const statusIconComponent = (status: string) => {
  if (status === "completed") return ListChecks;
  if (status === "in_progress") return ListMinus;
  return ListTodo;
};

const onPointerDownDocument = (event: MouseEvent) => {
  if (!isOpen.value || !rootRef.value) {
    return;
  }
  const target = event.target as Node | null;
  if (target && !rootRef.value.contains(target)) {
    isOpen.value = false;
  }
};

watch(
  () => props.conversationId,
  () => {
    todos.value = [];
    isOpen.value = false;
    // 切换会话后立即拉取，保证小框上的进度计数始终可见。
    fetchTodos();
  },
);

onMounted(async () => {
  document.addEventListener("mousedown", onPointerDownDocument);
  // 挂载时立即拉取一次：刷新页面后进度计数直接可见。
  fetchTodos();
  unlistenFn = await listen<{ conversationId: string | null }>(
    "todo-updated",
    (event) => {
      // 只刷新当前会话的待办，避免跨会话干扰。
      const incoming = event.payload.conversationId;
      const current = props.conversationId;
      const same =
        (incoming == null && current == null) ||
        (incoming != null && current != null && incoming === current);
      if (same) {
        fetchTodos();
      }
    },
  );
});

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onPointerDownDocument);
  if (unlistenFn) {
    unlistenFn();
  }
});
</script>

<template>
  <div ref="rootRef" class="relative inline-flex">
    <!-- 紧凑小框：与计划小框同规格（h-7 单行），有任务才显示 -->
    <button
      v-if="total > 0"
      type="button"
      class="inline-flex h-7 max-w-full items-center gap-1.5 rounded-lg border border-[#e7e9ee] bg-[#fafbfc] px-2 text-left transition-colors hover:bg-[#f3f5f8] dark:border-[#343434] dark:bg-white/5 dark:hover:bg-white/10"
      :title="`任务进度 ${done}/${total}`"
      @click="togglePanel"
    >
      <ListTodo class="h-3.5 w-3.5 shrink-0 text-[#64748b] dark:text-[#9ca3af]" />
      <span class="shrink-0 text-[11px] text-[#8a94a3] dark:text-[#858585]">任务进度</span>
      <span
        class="shrink-0 text-[12px] font-medium"
        :class="done === total
          ? 'text-[#24704f] dark:text-[#99d3b3]'
          : 'text-[#111827] dark:text-[#ececec]'"
      >
        {{ done }}/{{ total }}
      </span>
      <svg
        width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
        stroke-linecap="round" stroke-linejoin="round"
        class="shrink-0 text-[#94a3b8] transition-transform"
        :class="isOpen ? 'rotate-180' : ''"
      >
        <path d="M18 15l-6-6-6 6" />
      </svg>
    </button>

    <!-- 任务列表面板：向上弹出（小框在输入框上方，向下会挡住输入区） -->
    <div
      v-if="isOpen"
      class="absolute bottom-9 left-0 z-30 max-h-[52vh] w-[420px] max-w-[calc(100vw-64px)] overflow-hidden rounded-xl border border-[#e7e9ee] bg-white shadow-[0_12px_40px_rgba(15,23,42,0.14)] dark:border-[#343434] dark:bg-[#242424] dark:shadow-[0_12px_40px_rgba(0,0,0,0.4)]"
    >
      <!-- Header -->
      <div class="flex h-10 items-center justify-between gap-2 border-b border-[#eef0f3] px-3 dark:border-[#2c2c2c]">
        <span class="text-[13px] font-medium text-[#111827] dark:text-[#ececec]">任务进度</span>
        <span v-if="total > 0" class="text-[11px] text-[#8a94a3] dark:text-[#858585]">{{ done }}/{{ total }} 完成</span>
      </div>

      <!-- Empty state -->
      <div v-if="loading" class="px-3 py-5 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
        加载中...
      </div>
      <div v-else-if="total === 0" class="px-3 py-5 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
        当前会话还没有任务清单。
      </div>

      <!-- Todo list：每条仅左侧状态图标 + 内容，条目间细分隔线 -->
      <div v-else class="max-h-[42vh] overflow-y-auto px-3">
        <div
          v-for="(todo, index) in todos"
          :key="todo.id"
          class="flex items-start gap-2 py-2"
          :class="index > 0 ? 'border-t border-[#eef0f3] dark:border-[#2c2c2c]' : ''"
        >
          <component
            :is="statusIconComponent(todo.status)"
            class="mt-px h-3.5 w-3.5 shrink-0"
            :class="todo.status === 'completed'
              ? 'text-[#24704f] dark:text-[#99d3b3]'
              : todo.status === 'in_progress'
                ? 'text-[#315f8f] dark:text-[#bfdbfe]'
                : 'text-[#98a2b3] dark:text-[#b2aa9d]'"
          />
          <span
            class="min-w-0 flex-1 break-words text-[12px] leading-snug"
            :class="todo.status === 'completed'
              ? 'line-through text-[#64748b] dark:text-[#94a3b8]'
              : 'text-[#111827] dark:text-[#ececec]'"
          >{{ todo.content }}</span>
        </div>
      </div>
    </div>
  </div>
</template>
