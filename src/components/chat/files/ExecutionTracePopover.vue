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
  running: "text-[#2563eb] dark:text-[#60a5fa]",
  completed: "text-[#16a34a] dark:text-[#4ade80]",
  error: "text-[#dc2626] dark:text-[#f87171]",
  cancelled: "text-[#9ca3af] dark:text-[#9ca3af]",
};

const statusDotClassMap: Record<ToolExecutionEntry["status"], string> = {
  running: "bg-[#2563eb] animate-pulse",
  completed: "bg-[#16a34a]",
  error: "bg-[#dc2626]",
  cancelled: "bg-[#9ca3af]",
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
      class="absolute right-0 top-10 w-[420px] max-h-[68vh] overflow-hidden rounded-xl border border-[#e5e7eb] bg-[#fafafa] shadow-[0_16px_48px_rgba(15,23,42,0.14)] dark:border-[#3f3f3f] dark:bg-[#1c1c1c]"
    >
      <div class="flex items-center justify-between border-b border-[#e5e7eb] bg-white px-3.5 py-2.5 dark:border-[#333] dark:bg-[#242424]">
        <span class="text-[13px] font-semibold text-[#111827] dark:text-[#ececec]">AI 执行日志</span>
        <span class="rounded-full bg-[#f3f4f6] px-2 py-0.5 text-[11px] font-medium tabular-nums text-[#6b7280] dark:bg-[#2e2e2e] dark:text-[#a3a3a3]">
          {{ props.entries.length }} 条
        </span>
      </div>

      <div v-if="props.entries.length === 0" class="px-4 py-10 text-center text-[12px] text-[#94a3b8] dark:text-[#737373]">
        当前会话还没有工具执行记录。
      </div>

      <div v-else class="max-h-[60vh] space-y-2 overflow-y-auto p-2.5">
        <div
          v-for="entry in displayedEntries"
          :key="entry.id"
          class="overflow-hidden rounded-lg border border-[#e5e7eb] bg-white shadow-[0_1px_2px_rgba(15,23,42,0.04)] dark:border-[#383838] dark:bg-[#262626]"
        >
          <button
            type="button"
            class="flex w-full cursor-pointer items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-[#fafafa] dark:hover:bg-white/[0.03]"
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
              class="shrink-0 text-[#94a3b8] transition-transform duration-200"
              :class="isEntryCollapsed(entry.id) ? '' : 'rotate-90'"
            >
              <polyline points="9 18 15 12 9 6" />
            </svg>
            <span class="truncate text-[12.5px] font-semibold text-[#111827] dark:text-[#ececec]">{{ entry.toolName }}</span>
            <span class="inline-flex shrink-0 items-center gap-1 text-[11px] font-medium" :class="statusTextClassMap[entry.status]">
              <span class="h-1.5 w-1.5 rounded-full" :class="statusDotClassMap[entry.status]" />
              {{ statusLabelMap[entry.status] }}
            </span>
            <span class="ml-auto shrink-0 text-[10.5px] tabular-nums text-[#9ca3af] dark:text-[#737373]">{{ formatTime(entry.startedAt) }}</span>
          </button>

          <div class="border-t border-[#f1f5f9] px-3 pb-2.5 pt-2 dark:border-[#333]">
            <div v-if="isEntryCollapsed(entry.id)">
              <div class="mb-1 text-[10px] font-medium uppercase tracking-[0.05em] text-[#9ca3af] dark:text-[#737373]">预览</div>
              <div class="line-clamp-2 break-all rounded-md bg-[#f6f8fa] px-2.5 py-1.5 font-mono text-[11px] leading-relaxed text-[#475569] dark:bg-[#2e2e2e] dark:text-[#cbd5e1]">
                {{ collapsedPreview(entry) }}
              </div>
            </div>

            <template v-else>
              <div class="mb-1 text-[10px] font-medium uppercase tracking-[0.05em] text-[#9ca3af] dark:text-[#737373]">命令参数</div>
              <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md bg-[#f6f8fa] px-2.5 py-1.5 font-mono text-[11px] leading-relaxed text-[#334155] dark:bg-[#2e2e2e] dark:text-[#e5e7eb]">{{ inputText(entry) }}</pre>

              <div class="mb-1 mt-2.5 text-[10px] font-medium uppercase tracking-[0.05em] text-[#9ca3af] dark:text-[#737373]">执行结果</div>
              <pre class="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md bg-[#f6f8fa] px-2.5 py-1.5 font-mono text-[11px] leading-relaxed text-[#334155] dark:bg-[#2e2e2e] dark:text-[#e5e7eb]">{{ resultText(entry) }}</pre>
            </template>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
