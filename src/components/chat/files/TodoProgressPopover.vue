<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface TodoEntry {
  id: string;
  content: string;
  status: string;
  priority: string;
}

const props = defineProps<{
  conversationId: string | null;
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

const statusIcon = (status: string) => {
  if (status === "completed") return "[x]";
  if (status === "in_progress") return "[>]";
  return "[ ]";
};

const priorityLabel = (priority: string) => {
  if (priority === "high") return "HIGH";
  if (priority === "low") return "LOW";
  return "MED";
};

const priorityClass = (priority: string) => {
  if (priority === "high") return "text-[#9b3c35] dark:text-[#f0a8a1]";
  if (priority === "low") return "text-[#64748b] dark:text-[#94a3b8]";
  return "text-[#b58605] dark:text-[#d4a72c]";
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
    // 切换会话后立即拉取，保证徽标上的进度计数始终可见，
    // 而不是要等用户点开面板才加载。
    fetchTodos();
  },
);

onMounted(async () => {
  document.addEventListener("mousedown", onPointerDownDocument);
  // 挂载时立即拉取一次：刷新页面后徽标（done/total）直接可见，
  // 不需要点开面板才能看到任务进度。
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
  <div ref="rootRef" class="relative pointer-events-auto">
    <Button
      variant="outline"
      size="sm"
      class="h-8 px-1.5 rounded-md border-transparent bg-transparent text-[#4f5f73] dark:text-[#d5dbe3] inline-flex items-center gap-1.5 shadow-none hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
      :class="{ 'bg-black/5 dark:bg-white/10': isOpen }"
      @click="togglePanel"
      :title="total > 0 ? `任务进度 ${done}/${total}` : '任务清单'"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M9 11l3 3L22 4" />
        <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
      </svg>
      <!-- 轻量计数：纯文字，无背景徽标 -->
      <span
        v-if="total > 0"
        class="text-[11px] leading-none"
        :class="done === total
          ? 'text-[#24704f] dark:text-[#99d3b3]'
          : 'text-[#8a94a6] dark:text-[#9aa3b2]'"
      >
        {{ done }}/{{ total }}
      </span>
    </Button>

    <div
      v-if="isOpen"
      class="absolute right-0 top-10 w-[420px] max-h-[68vh] overflow-hidden rounded-2xl border border-[#e8e5df] dark:border-[#464646] bg-white dark:bg-[#242424] shadow-[0_18px_56px_rgba(32,28,24,0.12)]"
    >
      <!-- Header -->
      <div class="px-3 py-2.5 border-b border-[#eeeae3] dark:border-[#3a3a3a] flex items-center justify-between gap-2">
        <span class="text-[12px] font-medium text-[#111827] dark:text-[#e2dbcf]">任务进度</span>
        <span v-if="total > 0" class="text-[11px] text-[#667085] dark:text-[#cbd5e1]">{{ done }}/{{ total }} 完成</span>
      </div>

      <!-- Empty state -->
      <div v-if="loading" class="px-3 py-5 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
        加载中...
      </div>
      <div v-else-if="total === 0" class="px-3 py-5 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
        当前会话还没有任务清单。
      </div>

      <!-- Todo list：轻量卡片，细边框 + 淡背景区分每条任务 -->
      <div v-else class="max-h-[52vh] overflow-y-auto px-2.5 py-2 space-y-1.5">
        <div
          v-for="(todo, idx) in todos"
          :key="todo.id"
          class="rounded-lg border border-[#eceae5] bg-[#fafaf8] px-2.5 py-2 flex items-start gap-2 dark:border-[#383838] dark:bg-[#2a2a2a]"
        >
          <span
            class="font-mono text-[12px] mt-0.5 shrink-0"
            :class="todo.status === 'completed'
              ? 'text-[#24704f] dark:text-[#99d3b3]'
              : todo.status === 'in_progress'
                ? 'text-[#315f8f] dark:text-[#bfdbfe]'
                : 'text-[#98a2b3] dark:text-[#b2aa9d]'"
          >{{ statusIcon(todo.status) }}</span>
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <span class="text-[10px] text-[#98a2b3] dark:text-[#9d9589] shrink-0">#{{ idx + 1 }}</span>
              <span
              class="text-[12px] leading-snug break-words"
              :class="todo.status === 'completed'
                ? 'line-through text-[#64748b] dark:text-[#94a3b8]'
                : 'text-[#111827] dark:text-[#e2dbcf]'"
              >{{ todo.content }}</span>
            </div>
            <div class="mt-0.5 flex items-center gap-2">
              <span class="text-[10px] font-medium" :class="priorityClass(todo.priority)">{{ priorityLabel(todo.priority) }}</span>
              <span class="text-[11px] font-medium" :class="todo.status === 'completed' ? 'text-[#24704f] dark:text-[#99d3b3]' : todo.status === 'in_progress' ? 'text-[#315f8f] dark:text-[#bfdbfe]' : 'text-[#98a2b3] dark:text-[#9d9589]'">
                {{ todo.status === 'completed' ? '已完成' : todo.status === 'in_progress' ? '进行中' : '待开始' }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
