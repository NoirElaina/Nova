<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import {
  getFileChange,
  listFileChanges,
  revertFileChange,
  type FileChangeBatch,
  type FileChangeBatchSummary,
  type FileChangeEntry,
  type FileDiffLine,
} from "@/features/chat/services/chat-api";
import { emitToast } from "@/lib/toast";

const props = defineProps<{
  conversationId?: string | null;
}>();

const batches = ref<FileChangeBatchSummary[]>([]);
const batchDetails = ref<Record<string, FileChangeBatch>>({});
const loadingDetailIds = ref<Set<string>>(new Set());
const detailErrors = ref<Record<string, string>>({});
const loading = ref(false);
const revertingId = ref<string | null>(null);
const error = ref("");
const expandedBatchIds = ref<Set<string>>(new Set());
const expandedFileIds = ref<Set<string>>(new Set());
let refreshTimer: number | null = null;

const activeBatches = computed(() => batches.value.filter((batch) => !batch.reverted));
const hasChanges = computed(() => batches.value.length > 0);
const hasCurrentChanges = computed(() => activeBatches.value.length > 0);
const activeFileCount = computed(() =>
  activeBatches.value.reduce((total, batch) => total + batch.fileCount, 0),
);

const loadChanges = async () => {
  loading.value = true;
  error.value = "";
  try {
    batches.value = await listFileChanges(props.conversationId ?? null);
  } catch (err) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
};

const formatTime = (value: number) => {
  return new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date(value));
};

const formatChangeType = (type: FileChangeEntry["changeType"]) => {
  if (type === "added") return "新增";
  if (type === "deleted") return "删除";
  return "修改";
};

const fileStats = (file: Pick<FileChangeEntry, "diff">) => {
  const added = file.diff.filter((line) => line.kind === "add").length;
  const removed = file.diff.filter((line) => line.kind === "remove").length;
  return { added, removed };
};

const batchStats = (batch: FileChangeBatchSummary) => ({
  added: batch.additions,
  removed: batch.deletions,
});

const lineClass = (line: FileDiffLine) => {
  if (line.kind === "add") return "bg-emerald-50 text-emerald-950 dark:bg-emerald-950/30 dark:text-emerald-100";
  if (line.kind === "remove") return "bg-rose-50 text-rose-950 dark:bg-rose-950/30 dark:text-rose-100";
  return "text-[#334155] dark:text-[#cbd5e1]";
};

const linePrefix = (line: FileDiffLine) => {
  if (line.kind === "add") return "+";
  if (line.kind === "remove") return "-";
  return " ";
};

const lineNumber = (line: FileDiffLine) => {
  return line.newLine ?? line.oldLine ?? "";
};

const isBatchExpanded = (batchId: string) => expandedBatchIds.value.has(batchId);

const isDetailLoading = (batchId: string) => loadingDetailIds.value.has(batchId);

const batchDetail = (batchId: string) => batchDetails.value[batchId];

const setDetailLoading = (batchId: string, loading: boolean) => {
  const next = new Set(loadingDetailIds.value);
  if (loading) {
    next.add(batchId);
  } else {
    next.delete(batchId);
  }
  loadingDetailIds.value = next;
};

const loadBatchDetail = async (batchId: string) => {
  if (batchDetails.value[batchId] || isDetailLoading(batchId)) return;
  setDetailLoading(batchId, true);
  detailErrors.value = { ...detailErrors.value, [batchId]: "" };
  try {
    const detail = await getFileChange(props.conversationId ?? null, batchId);
    batchDetails.value = { ...batchDetails.value, [batchId]: detail };
  } catch (err) {
    detailErrors.value = { ...detailErrors.value, [batchId]: String(err) };
  } finally {
    setDetailLoading(batchId, false);
  }
};

const toggleBatch = async (batchId: string) => {
  const next = new Set(expandedBatchIds.value);
  if (next.has(batchId)) {
    next.delete(batchId);
  } else {
    next.add(batchId);
  }
  expandedBatchIds.value = next;
  if (next.has(batchId)) {
    await loadBatchDetail(batchId);
  }
};

const fileId = (batch: Pick<FileChangeBatch, "id">, file: FileChangeEntry) =>
  `${batch.id}:${file.absolutePath || file.path}`;

const isFileExpanded = (id: string) => expandedFileIds.value.has(id);

const toggleFile = (id: string) => {
  const next = new Set(expandedFileIds.value);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  expandedFileIds.value = next;
};

const handleRevert = async (batch: FileChangeBatchSummary) => {
  if (batch.reverted || revertingId.value) return;
  revertingId.value = batch.id;
  error.value = "";
  try {
    await revertFileChange(props.conversationId ?? null, batch.id);
    const nextDetails = { ...batchDetails.value };
    delete nextDetails[batch.id];
    batchDetails.value = nextDetails;
    await loadChanges();
    emitToast({
      variant: "success",
      source: "file-review",
      message: "已回退本次文件变更。",
    });
  } catch (err) {
    const message = String(err);
    error.value = message;
    emitToast({
      variant: "error",
      source: "file-review",
      message,
    });
  } finally {
    revertingId.value = null;
  }
};

watch(
  () => props.conversationId,
  () => {
    expandedBatchIds.value = new Set();
    expandedFileIds.value = new Set();
    batchDetails.value = {};
    detailErrors.value = {};
    void loadChanges();
  },
  { immediate: true },
);

onMounted(() => {
  refreshTimer = window.setInterval(() => {
    if (!revertingId.value) void loadChanges();
  }, 2500);
});

onBeforeUnmount(() => {
  if (refreshTimer !== null) {
    window.clearInterval(refreshTimer);
  }
});
</script>

<template>
  <div class="flex h-full flex-col bg-white dark:bg-[#1e1e1e]">
    <div class="flex h-11 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-4 dark:border-[#333]">
      <div class="flex min-w-0 items-center gap-2">
        <span class="text-sm font-medium text-[#111827] dark:text-[#f8fafc]">文件变更</span>
        <span class="rounded-full bg-[#f3f4f6] px-2 py-0.5 text-[11px] text-[#64748b] dark:bg-[#2b2b2b] dark:text-[#cbd5e1]">
          {{ activeFileCount }} 文件 · {{ activeBatches.length }} 次
        </span>
      </div>
      <Button variant="ghost" size="sm" class="h-7 px-2 text-xs text-[#64748b]" :disabled="loading" @click="loadChanges">
        {{ loading ? "刷新中" : "刷新" }}
      </Button>
    </div>

    <div v-if="error" class="mx-4 mt-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/30 dark:text-rose-200">
      {{ error }}
    </div>

    <div v-if="!loading && !hasChanges" class="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-[#64748b] dark:text-[#94a3b8]">
      <svg width="38" height="38" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="opacity-45">
        <path d="M4 7h16" />
        <path d="M4 12h16" />
        <path d="M4 17h10" />
      </svg>
      <div>
        <div class="text-sm font-medium text-[#111827] dark:text-[#f8fafc]">暂无可审查变更</div>
        <p class="mt-1 text-xs">AI 使用 apply_patch 后会出现在这里。</p>
      </div>
    </div>

    <div v-else-if="!loading && !hasCurrentChanges" class="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-[#64748b] dark:text-[#94a3b8]">
      <div class="rounded-2xl border border-[#e5e7eb] bg-[#fafafa] px-5 py-4 dark:border-[#333] dark:bg-[#252525]">
        <div class="text-sm font-medium text-[#111827] dark:text-[#f8fafc]">当前没有未回退变更</div>
        <p class="mt-1 text-xs">历史审查记录仍保留，新的文件改动会继续显示在这里。</p>
      </div>
    </div>

    <div v-else class="min-h-0 flex-1 overflow-y-auto px-3 py-3">
      <div class="flex flex-col gap-3">
        <section
          v-for="batch in activeBatches"
          :key="batch.id"
          class="overflow-hidden rounded-xl border border-[#e5e7eb] bg-white dark:border-[#333] dark:bg-[#202020]"
        >
          <header
            class="flex cursor-pointer items-center justify-between gap-3 border-b border-[#eef0f3] px-3 py-2 transition-colors hover:bg-[#fafafa] dark:border-[#333] dark:hover:bg-[#262626]"
            @click="void toggleBatch(batch.id)"
          >
            <div class="min-w-0">
              <div class="flex min-w-0 items-center gap-2">
                <svg
                  width="13"
                  height="13"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  class="shrink-0 text-[#94a3b8] transition-transform duration-150"
                  :class="isBatchExpanded(batch.id) ? 'rotate-90' : ''"
                  aria-hidden="true"
                >
                  <polyline points="9 18 15 12 9 6" />
                </svg>
                <span class="truncate font-mono text-xs text-[#0f172a] dark:text-[#e2e8f0]">{{ batch.toolName }}</span>
                <span class="text-[11px] text-[#94a3b8]">{{ formatTime(batch.createdAt) }}</span>
              </div>
              <div class="mt-1 flex min-w-0 items-center gap-2 text-xs text-[#64748b] dark:text-[#94a3b8]">
                <span>{{ batch.fileCount }} 个文件</span>
                <span>·</span>
                <span class="text-emerald-600">+{{ batchStats(batch).added }}</span>
                <span class="text-rose-600">-{{ batchStats(batch).removed }}</span>
              </div>
            </div>
            <div class="flex shrink-0 items-center gap-1.5">
              <Button
                variant="ghost"
                size="sm"
                class="h-7 px-2 text-xs text-[#64748b]"
                @click.stop="void toggleBatch(batch.id)"
              >
                {{ isBatchExpanded(batch.id) ? "收起" : "展开" }}
              </Button>
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-2 text-xs"
                :disabled="batch.reverted || revertingId === batch.id"
                @click.stop="handleRevert(batch)"
              >
                {{ revertingId === batch.id ? "回退中" : "回退" }}
              </Button>
            </div>
          </header>

          <div v-if="isBatchExpanded(batch.id)" class="divide-y divide-[#eef0f3] bg-[#fcfcfd] dark:divide-[#333] dark:bg-[#1c1c1c]">
            <div v-if="isDetailLoading(batch.id)" class="px-3 py-4 text-xs text-[#64748b] dark:text-[#94a3b8]">
              正在加载 diff 详情...
            </div>
            <div v-else-if="detailErrors[batch.id]" class="px-3 py-4 text-xs text-rose-600 dark:text-rose-300">
              {{ detailErrors[batch.id] }}
            </div>
            <article
              v-for="file in batchDetail(batch.id)?.files || []"
              :key="fileId(batch, file)"
              class="bg-white dark:bg-[#202020]"
            >
              <header
                class="flex cursor-pointer items-center justify-between gap-3 px-3 py-2 transition-colors hover:bg-[#fafafa] dark:hover:bg-[#262626]"
                @click="toggleFile(fileId(batch, file))"
              >
                <div class="min-w-0">
                  <div class="flex min-w-0 items-center gap-2">
                    <svg
                      width="12"
                      height="12"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      class="shrink-0 text-[#94a3b8] transition-transform duration-150"
                      :class="isFileExpanded(fileId(batch, file)) ? 'rotate-90' : ''"
                      aria-hidden="true"
                    >
                      <polyline points="9 18 15 12 9 6" />
                    </svg>
                    <span class="truncate font-mono text-xs text-[#0f172a] dark:text-[#e2e8f0]">{{ file.path }}</span>
                  </div>
                  <div class="mt-1 flex min-w-0 items-center gap-2 text-xs text-[#64748b] dark:text-[#94a3b8]">
                    <span>{{ formatChangeType(file.changeType) }}</span>
                    <span>·</span>
                    <span class="text-emerald-600">+{{ fileStats(file).added }}</span>
                    <span class="text-rose-600">-{{ fileStats(file).removed }}</span>
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-7 px-2 text-xs text-[#64748b]"
                  @click.stop="toggleFile(fileId(batch, file))"
                >
                  {{ isFileExpanded(fileId(batch, file)) ? "收起" : "展开" }}
                </Button>
              </header>

              <div
                v-if="isFileExpanded(fileId(batch, file))"
                class="max-h-[360px] overflow-auto border-t border-[#f1f5f9] bg-[#fbfdff] font-mono text-[12px] leading-5 dark:border-[#2b2b2b] dark:bg-[#191919]"
              >
                <div
                  v-for="(line, index) in file.diff"
                  :key="`${fileId(batch, file)}:${index}`"
                  class="grid grid-cols-[44px_24px_minmax(0,1fr)] px-2"
                  :class="lineClass(line)"
                >
                  <span class="select-none text-right text-[#94a3b8]">{{ lineNumber(line) }}</span>
                  <span class="select-none text-center">{{ linePrefix(line) }}</span>
                  <span class="whitespace-pre">{{ line.text || " " }}</span>
                </div>
              </div>
            </article>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
