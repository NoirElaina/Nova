<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import {
  getGitRepoStatus,
  getWorkspaceDiff,
  initGitRepo,
  type FileDiffLine,
  type GitRepoStatus,
  type WorkspaceDiff,
  type WorkspaceFileChange,
} from "@/features/chat/services/chat-api";
import { emitToast } from "@/lib/toast";

const props = defineProps<{
  conversationId?: string | null;
}>();

const diff = ref<WorkspaceDiff | null>(null);
const loading = ref(false);
const error = ref("");
const expandedFileIds = ref<Set<string>>(new Set());
let refreshTimer: number | null = null;

// 工作区 git 初始化状态。默认流程不再自动 `git init`，
// 必须由用户点击「初始化 Git」按钮显式触发，避免污染用户工作目录。
const gitStatus = ref<GitRepoStatus | null>(null);
const gitStatusLoading = ref(false);
const initializingGit = ref(false);

const files = computed(() => diff.value?.files ?? []);
const hasChanges = computed(() => files.value.length > 0);
const totalAdditions = computed(() => diff.value?.totalAdditions ?? 0);
const totalDeletions = computed(() => diff.value?.totalDeletions ?? 0);

const loadChanges = async () => {
  loading.value = true;
  error.value = "";
  try {
    diff.value = await getWorkspaceDiff(props.conversationId ?? null);
  } catch (err) {
    error.value = String(err);
  } finally {
    loading.value = false;
  }
};

const loadGitStatus = async () => {
  gitStatusLoading.value = true;
  try {
    gitStatus.value = await getGitRepoStatus(props.conversationId ?? null);
  } catch (err) {
    // 查询失败不阻断主流程，只是按钮状态无法精确显示。
    gitStatus.value = null;
    console.warn("[CodeDiffTab] getGitRepoStatus failed:", err);
  } finally {
    gitStatusLoading.value = false;
  }
};

const handleInitGit = async () => {
  if (initializingGit.value) return;
  initializingGit.value = true;
  error.value = "";
  try {
    const result = await initGitRepo(props.conversationId ?? null);
    await loadGitStatus();
    await loadChanges();
    emitToast({
      variant: "success",
      source: "file-review",
      message: result.created
        ? `已在该会话工作区初始化 Git 仓库：${result.path}`
        : `该工作区已经是 Git 仓库：${result.path}`,
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
    initializingGit.value = false;
  }
};

const gitInitialized = computed(() => gitStatus.value?.initialized === true);

const formatChangeType = (type: WorkspaceFileChange["changeType"]) => {
  if (type === "added") return "新增";
  if (type === "deleted") return "删除";
  return "修改";
};

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

const fileKey = (file: WorkspaceFileChange) =>
  file.absolutePath || file.path;

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

watch(
  () => props.conversationId,
  () => {
    expandedFileIds.value = new Set();
    void loadChanges();
    void loadGitStatus();
  },
  { immediate: true },
);

onMounted(() => {
  refreshTimer = window.setInterval(() => {
    void loadChanges();
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
        <span class="text-sm font-medium text-[#111827] dark:text-[#f8fafc]">审查</span>
        <span class="rounded-full bg-[#f3f4f6] px-2 py-0.5 text-[11px] text-[#64748b] dark:bg-[#2b2b2b] dark:text-[#cbd5e1]">
          {{ files.length }} 文件 · +{{ totalAdditions }} -{{ totalDeletions }}
        </span>
      </div>
      <div class="flex shrink-0 items-center gap-1.5">
        <Button
          variant="ghost"
          size="sm"
          class="h-7 px-2 text-xs text-[#64748b]"
          :disabled="loading"
          @click="loadChanges"
        >
          {{ loading ? "刷新中" : "刷新" }}
        </Button>
        <Button
          v-if="!gitInitialized"
          variant="outline"
          size="sm"
          class="h-7 px-2 text-xs"
          :disabled="initializingGit || gitStatusLoading"
          :title="gitStatus ? `工作区：${gitStatus.path}` : '在该工作区创建 Git 仓库以启用审查'"
          @click="handleInitGit"
        >
          {{ initializingGit ? "初始化中" : "初始化 Git" }}
        </Button>
        <span
          v-else
          class="inline-flex items-center gap-1 rounded-md bg-emerald-50 px-2 py-0.5 text-[11px] text-emerald-700 dark:bg-emerald-950/30 dark:text-emerald-300"
          :title="gitStatus ? `工作区：${gitStatus.path}` : '工作区已是 Git 仓库'"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <polyline points="20 6 9 17 4 12" />
          </svg>
          Git 已就绪
        </span>
      </div>
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
        <div class="text-sm font-medium text-[#111827] dark:text-[#f8fafc]">暂无未提交改动</div>
        <p class="mt-1 text-xs">把工作区初始化为 Git 仓库后，工作区相对 HEAD 的改动会实时展示在这里。</p>
      </div>
    </div>

    <div v-else class="min-h-0 flex-1 overflow-y-auto px-3 py-3">
      <div class="flex flex-col gap-3">
        <section
          v-for="file in files"
          :key="fileKey(file)"
          class="overflow-hidden rounded-xl border border-[#e5e7eb] bg-white dark:border-[#333] dark:bg-[#202020]"
        >
          <header
            class="flex cursor-pointer items-center justify-between gap-3 px-3 py-2 transition-colors hover:bg-[#fafafa] dark:hover:bg-[#262626]"
            @click="toggleFile(fileKey(file))"
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
                  :class="isFileExpanded(fileKey(file)) ? 'rotate-90' : ''"
                  aria-hidden="true"
                >
                  <polyline points="9 18 15 12 9 6" />
                </svg>
                <span class="truncate font-mono text-xs text-[#0f172a] dark:text-[#e2e8f0]">{{ file.path }}</span>
              </div>
              <div class="mt-1 flex min-w-0 items-center gap-2 text-xs text-[#64748b] dark:text-[#94a3b8]">
                <span>{{ formatChangeType(file.changeType) }}</span>
                <span>·</span>
                <span class="text-emerald-600">+{{ file.additions }}</span>
                <span class="text-rose-600">-{{ file.deletions }}</span>
              </div>
            </div>
            <Button
              variant="ghost"
              size="sm"
              class="h-7 px-2 text-xs text-[#64748b]"
              @click.stop="toggleFile(fileKey(file))"
            >
              {{ isFileExpanded(fileKey(file)) ? "收起" : "展开" }}
            </Button>
          </header>

          <div
            v-if="isFileExpanded(fileKey(file))"
            class="max-h-[360px] overflow-auto border-t border-[#f1f5f9] bg-[#fbfdff] font-mono text-[12px] leading-5 dark:border-[#2b2b2b] dark:bg-[#191919]"
          >
            <div
              v-for="(line, index) in file.diff"
              :key="`${fileKey(file)}:${index}`"
              class="grid grid-cols-[44px_24px_minmax(0,1fr)] px-2"
              :class="lineClass(line)"
            >
              <span class="select-none text-right text-[#94a3b8]">{{ lineNumber(line) }}</span>
              <span class="select-none text-center">{{ linePrefix(line) }}</span>
              <span class="whitespace-pre">{{ line.text || " " }}</span>
            </div>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
