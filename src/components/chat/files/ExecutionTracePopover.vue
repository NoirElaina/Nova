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
  const text = (entry.result || entry.input || "").trim();
  if (!text) {
    return "（无可预览内容）";
  }
  return text.length > 100 ? `${text.slice(0, 100)}...` : text;
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

const statusClassMap: Record<ToolExecutionEntry["status"], string> = {
  running: "trace-status-running",
  completed: "trace-status-completed",
  error: "trace-status-error",
  cancelled: "trace-status-cancelled",
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
      class="h-8 px-3 rounded-md border border-[#e5e7eb] dark:border-[#444] bg-white/95 dark:bg-[#262626] text-[12px] text-[#475569] dark:text-[#d5dbe3] inline-flex items-center gap-2 hover:bg-[#f8fafc] dark:hover:bg-[#2f2f2f] transition-colors"
      @click="togglePanel"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 12h18" />
        <path d="M3 6h18" />
        <path d="M3 18h18" />
      </svg>
      执行日志
      <span class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 rounded-full bg-[#eef2f7] dark:bg-[#334155] text-[11px] leading-none">
        {{ props.entries.length }}
      </span>
    </Button>

    <div
      v-if="isOpen"
      class="absolute right-0 top-10 w-[420px] max-h-[68vh] overflow-hidden rounded-2xl border border-[#e5e7eb] dark:border-[#464646] bg-white dark:bg-[#242424] shadow-[0_18px_56px_rgba(15,23,42,0.16)]"
    >
      <div class="px-3 py-2.5 border-b border-[#e5e7eb] dark:border-[#3a3a3a] text-[12px] text-[#64748b] dark:text-[#cbd5e1] flex items-center justify-between">
        <span class="font-medium">AI 执行日志</span>
        <span>{{ props.entries.length }} 条</span>
      </div>

      <div v-if="props.entries.length === 0" class="px-3 py-5 text-[12px] text-[#94a3b8] dark:text-[#9b9489]">
        当前会话还没有工具执行记录。
      </div>

      <div v-else class="max-h-[60vh] overflow-y-auto px-2.5 py-2 space-y-2">
        <div
          v-for="entry in displayedEntries"
          :key="entry.id"
          class="rounded-xl border border-[#e5e7eb] dark:border-[#3a3a3a] bg-[#f8fafc] dark:bg-[#2b2b2b] px-3 py-2.5"
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
                class="text-[#94a3b8] dark:text-[#b2aa9d] transition-transform duration-200"
                :class="isEntryCollapsed(entry.id) ? '' : 'rotate-90'"
              >
                <polyline points="9 18 15 12 9 6" />
              </svg>
              <span class="text-[12px] font-medium text-[#111827] dark:text-[#e2dbcf] truncate">
                {{ entry.toolName }}
              </span>
            </Button>
            <div class="inline-flex items-center gap-1">
              <span class="trace-status" :class="statusClassMap[entry.status]">{{ statusLabelMap[entry.status] }}</span>
              <span class="text-[10px] text-[#94a3b8] dark:text-[#9d9589] shrink-0">{{ formatTime(entry.startedAt) }}</span>
            </div>
          </div>

          <div
            v-if="isEntryCollapsed(entry.id)"
            class="mt-2 text-[11px] text-[#64748b] dark:text-[#ada496]"
          >
            <div class="font-medium mb-1">预览</div>
            <div class="trace-collapsed-preview">{{ collapsedPreview(entry) }}</div>
          </div>

          <template v-else>
            <div class="mt-2 text-[11px] text-[#64748b] dark:text-[#ada496]">
              <div class="font-medium mb-1">命令参数</div>
              <pre class="trace-content">{{ entry.input || '（无参数）' }}</pre>
            </div>

            <div class="mt-2 text-[11px] text-[#64748b] dark:text-[#ada496]">
              <div class="font-medium mb-1">执行结果</div>
              <pre class="trace-content">{{ entry.result || '（暂无结果）' }}</pre>
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
  color: #64748b;
  border: 1px dashed #cbd5e1;
  border-radius: 8px;
  padding: 6px 8px;
  background: rgba(255, 255, 255, 0.55);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dark .trace-collapsed-preview {
  color: #cbd5e1;
  border-color: #475569;
  background: rgba(0, 0, 0, 0.18);
}

.trace-content {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "SF Mono", "Fira Code", "Cascadia Mono", monospace;
  background: rgba(255, 255, 255, 0.6);
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  padding: 6px 8px;
  max-height: 160px;
  overflow: auto;
}

.dark .trace-content {
  background: rgba(0, 0, 0, 0.2);
  border-color: #434343;
}

.trace-status {
  font-size: 10px;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 999px;
  border: 1px solid transparent;
}

.trace-status-running {
  color: #1d4ed8;
  background: #eff6ff;
  border-color: #bfdbfe;
}

.trace-status-completed {
  color: #2f6b4e;
  background: #e9f7ef;
  border-color: #bfe2cb;
}

.trace-status-error {
  color: #9b3c35;
  background: #fdebea;
  border-color: #efc5c2;
}

.trace-status-cancelled {
  color: #64748b;
  background: #f1f5f9;
  border-color: #cbd5e1;
}

.dark .trace-status-running {
  color: #bfdbfe;
  background: #172554;
  border-color: #1e40af;
}

.dark .trace-status-completed {
  color: #99d3b3;
  background: #1f3b2e;
  border-color: #315845;
}

.dark .trace-status-error {
  color: #f0a8a1;
  background: #4a2723;
  border-color: #6a3732;
}

.dark .trace-status-cancelled {
  color: #cbd5e1;
  background: #1e293b;
  border-color: #475569;
}
</style>
