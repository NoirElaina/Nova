<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { Button } from "@/components/ui/button";
import type { ToolExecutionEntry } from "../../../lib/chat-types";

const props = defineProps<{
  entries: ToolExecutionEntry[];
}>();

const rootRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);
const expandedEntryIds = ref<Set<string>>(new Set());

const togglePanel = () => {
  isOpen.value = !isOpen.value;
};

const isEntryCollapsed = (entryId: string) => !expandedEntryIds.value.has(entryId);

const toggleEntryCollapse = (entryId: string) => {
  const next = new Set(expandedEntryIds.value);
  if (next.has(entryId)) {
    next.delete(entryId);
  } else {
    next.add(entryId);
  }
  expandedEntryIds.value = next;
};

const collapsedPreview = (entry: ToolExecutionEntry) => {
  if (entry.status === "running" && !entry.input.trim() && !entry.result.trim()) {
    return "正在等待工具参数...";
  }
  const text = (entry.result || entry.input || "").trim();
  if (!text) {
    return "（无可预览内容）";
  }
  return text.length > 100 ? `${text.slice(0, 100)}...` : text;
};

const inputText = (entry: ToolExecutionEntry) => {
  if (entry.input.trim()) {
    return entry.input;
  }
  return entry.status === "running" ? "正在等待工具参数..." : "（无参数）";
};

const resultText = (entry: ToolExecutionEntry) => {
  if (entry.result.trim()) {
    return entry.result;
  }
  return entry.status === "running" ? "工具正在执行，等待结果..." : "（暂无结果）";
};

const displayedEntries = computed(() =>
  [...props.entries].sort((a, b) => {
    const timeA = a.finishedAt ?? a.startedAt ?? 0;
    const timeB = b.finishedAt ?? b.startedAt ?? 0;
    return timeB - timeA;
  }),
);

const statusLabelMap: Record<ToolExecutionEntry["status"], string> = {
  running: "执行中",
  completed: "已完成",
  error: "错误",
  cancelled: "已取消",
};

const statusTextClassMap: Record<ToolExecutionEntry["status"], string> = {
  running: "text-[#315f8f] dark:text-[#bfdbfe]",
  completed: "text-[#24704f] dark:text-[#99d3b3]",
  error: "text-[#9b3c35] dark:text-[#f0a8a1]",
  cancelled: "text-[#98a2b3] dark:text-[#9d9589]",
};

const formatTime = (ts: number) => {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) {
    return "--";
  }
  return date.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
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

onMounted(() => {
  document.addEventListener("mousedown", onPointerDownDocument);
});

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onPointerDownDocument);
});
</script>

<template>
  <div ref="rootRef" class="relative pointer-events-auto">
    <Button
      variant="outline"
      size="sm"
      class="h-8 px-1.5 rounded-md border-transparent bg-transparent text-[#4f5f73] dark:text-[#d5dbe3] inline-flex items-center gap-1.5 shadow-none hover:bg-black/5 dark:hover:bg-white/5 transition-colors"
      @click="togglePanel"
      title="AI 执行日志"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 12h18" />
        <path d="M3 6h18" />
        <path d="M3 18h18" />
      </svg>
      <!-- 轻量计数：纯文字，无背景徽标 -->
      <span class="text-[11px] leading-none text-[#8a94a6] dark:text-[#9aa3b2]">
        {{ props.entries.length }}
      </span>
    </Button>

    <div
      v-if="isOpen"
      class="absolute right-0 top-10 w-[420px] max-h-[68vh] overflow-hidden rounded-2xl border border-[#e8e5df] dark:border-[#464646] bg-white dark:bg-[#242424] shadow-[0_18px_56px_rgba(32,28,24,0.12)]"
    >
      <div class="px-3 py-2.5 border-b border-[#eeeae3] dark:border-[#3a3a3a] text-[12px] text-[#667085] dark:text-[#cbd5e1] flex items-center justify-between">
        <span class="font-medium">AI 执行日志</span>
        <span>{{ props.entries.length }} 条</span>
      </div>

      <div v-if="props.entries.length === 0" class="px-3 py-5 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
        当前会话还没有工具执行记录。
      </div>

      <div v-else class="max-h-[60vh] overflow-y-auto px-2.5 py-2 space-y-1.5">
        <div
          v-for="entry in displayedEntries"
          :key="entry.id"
          class="rounded-lg border border-[#eceae5] bg-[#fafaf8] px-2.5 py-2 dark:border-[#383838] dark:bg-[#2a2a2a]"
        >
          <div class="flex items-center justify-between gap-2">
            <Button
              variant="ghost"
              size="sm"
              type="button"
              class="h-auto flex-1 min-w-0 justify-start gap-1.5 px-0 text-left"
              :aria-expanded="!isEntryCollapsed(entry.id)"
              @click="toggleEntryCollapse(entry.id)"
            >
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                class="text-[#98a2b3] dark:text-[#b2aa9d] transition-transform duration-200"
                :class="isEntryCollapsed(entry.id) ? '' : 'rotate-90'"
              >
                <polyline points="9 18 15 12 9 6" />
              </svg>
              <span class="text-[13px] font-semibold text-[#111827] dark:text-[#e2dbcf] truncate">
                {{ entry.toolName }}
              </span>
            </Button>
            <div class="inline-flex items-center gap-2 shrink-0">
              <span class="text-[11px] font-medium" :class="statusTextClassMap[entry.status]">{{ statusLabelMap[entry.status] }}</span>
              <span class="text-[10px] text-[#98a2b3] dark:text-[#9d9589]">{{ formatTime(entry.startedAt) }}</span>
            </div>
          </div>

          <div
            v-if="isEntryCollapsed(entry.id)"
            class="mt-2 text-[11px] text-[#667085] dark:text-[#ada496]"
          >
            <div class="font-medium mb-1">预览</div>
            <div class="trace-collapsed-preview">{{ collapsedPreview(entry) }}</div>
          </div>

          <template v-else>
            <div class="mt-2 text-[11px] text-[#667085] dark:text-[#ada496]">
              <div class="font-medium mb-1">命令参数</div>
              <pre class="trace-content">{{ inputText(entry) }}</pre>
            </div>

            <div class="mt-2 text-[11px] text-[#667085] dark:text-[#ada496]">
              <div class="font-medium mb-1">执行结果</div>
              <pre class="trace-content">{{ resultText(entry) }}</pre>
            </div>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.trace-collapsed-preview {
  font-size: 11px;
  line-height: 1.5;
  color: #4b5563;
  background: rgba(0, 0, 0, 0.03);
  border-radius: 6px;
  padding: 4px 8px;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-all;
}

.dark .trace-collapsed-preview {
  color: #cbd5e1;
  background: rgba(255, 255, 255, 0.05);
}

.trace-content {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "SF Mono", "Fira Code", "Cascadia Mono", monospace;
  color: #334155;
  background: rgba(0, 0, 0, 0.03);
  border-radius: 6px;
  padding: 6px 8px;
  max-height: 160px;
  overflow: auto;
}

.dark .trace-content {
  color: #cbd5e1;
  background: rgba(255, 255, 255, 0.05);
}

</style>
