<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import hljs from "highlight.js";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { emitToast } from "../../../lib/toast";
import {
  listWorkspaceDirectory,
  readWorkspaceTextFile,
  setWorkspaceRoot,
  type WorkspaceDirectoryListing,
  type WorkspaceEntry,
  type WorkspaceFileContent,
} from "../../../features/workspace/workspace-api";
import WorkspaceFileTreeNode from "./WorkspaceFileTreeNode.vue";

const props = defineProps<{
  conversationId?: string | null;
}>();

const rootListing = ref<WorkspaceDirectoryListing | null>(null);
const childrenByPath = ref<Record<string, WorkspaceEntry[]>>({});
const expandedPaths = ref<string[]>([]);
const loadingPaths = ref<string[]>([]);
const filterQuery = ref("");
const selectedFile = ref<WorkspaceEntry | null>(null);
const selectedContent = ref<WorkspaceFileContent | null>(null);
const previewError = ref("");
const isReadingFile = ref(false);
const rootError = ref("");
const workspaceBodyRef = ref<HTMLElement | null>(null);
const moreMenuRef = ref<HTMLElement | null>(null);
const fileTreeWidth = ref(280);
const isFileTreeVisible = ref(true);
const isResizingFileTree = ref(false);
const isMoreMenuOpen = ref(false);
const isPreviewWrapEnabled = ref(false);
const isChangingWorkspace = ref(false);

const FILE_TREE_MIN_WIDTH = 220;
const FILE_TREE_MAX_WIDTH = 420;
const PREVIEW_MIN_WIDTH = 260;

let resizeStartX = 0;
let resizeStartWidth = 0;
let previousBodyCursor = "";
let previousBodyUserSelect = "";

const rootEntries = computed(() => childrenByPath.value[""] ?? []);
const selectedLines = computed(() => (selectedContent.value?.content ?? "").split(/\r?\n/));
const selectedLanguage = computed(() => {
  const extension = selectedFile.value?.extension?.toLowerCase();
  if (!extension) return "";
  const languageByExtension: Record<string, string> = {
    css: "css",
    html: "xml",
    js: "javascript",
    jsx: "javascript",
    json: "json",
    jsonc: "json",
    md: "markdown",
    mdx: "markdown",
    rs: "rust",
    toml: "toml",
    ts: "typescript",
    tsx: "typescript",
    vue: "xml",
  };
  return languageByExtension[extension] ?? extension;
});

const highlightClassStyles: Record<string, string> = {
  "hljs-attr": "color:#d93025",
  "hljs-built_in": "color:#8250df",
  "hljs-comment": "color:#6a737d",
  "hljs-keyword": "color:#cf222e",
  "hljs-literal": "color:#0550ae",
  "hljs-meta": "color:#6a737d",
  "hljs-number": "color:#0550ae",
  "hljs-property": "color:#d93025",
  "hljs-punctuation": "color:#57606a",
  "hljs-string": "color:#0a7f37",
  "hljs-title": "color:#8250df",
  "hljs-type": "color:#8250df",
  "hljs-variable": "color:#953800",
};

const escapeHtml = (value: string) =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");

const applyInlineHighlightStyles = (html: string) =>
  html.replace(/class="([^"]*hljs-[^"]*)"/g, (_match, className: string) => {
    const styles = className
      .split(/\s+/)
      .map((name) => highlightClassStyles[name])
      .filter(Boolean)
      .join(";");
    return styles ? `style="${styles}"` : "";
  });

const highlightLine = (line: string) => {
  if (!line) return "&nbsp;";
  const language = selectedLanguage.value;
  try {
    if (!language || !hljs.getLanguage(language)) {
      return escapeHtml(line);
    }
    const result = hljs.highlight(line, { language, ignoreIllegals: true });
    return applyInlineHighlightStyles(result.value);
  } catch {
    return escapeHtml(line);
  }
};

const selectedPreviewLines = computed(() => selectedLines.value.map(highlightLine));
const previewCodeGridClass = computed(() =>
  isPreviewWrapEnabled.value ? "w-full" : "min-w-max",
);
const previewCodeLineClass = computed(() =>
  isPreviewWrapEnabled.value ? "whitespace-pre-wrap break-words" : "whitespace-pre",
);
const fileTreePaneStyle = computed(() => ({
  width: isFileTreeVisible.value ? `${fileTreeWidth.value}px` : "0px",
}));

const normalizedFilter = computed(() => filterQuery.value.trim().toLowerCase());

const entryMatchesFilter = (entry: WorkspaceEntry): boolean => {
  if (!normalizedFilter.value) return true;
  if (entry.name.toLowerCase().includes(normalizedFilter.value)) return true;
  const children = childrenByPath.value[entry.relativePath] ?? [];
  return children.some(entryMatchesFilter);
};

const visibleRootEntries = computed(() => rootEntries.value.filter(entryMatchesFilter));

const fileIconLabel = computed(() => {
  const extension = selectedFile.value?.extension?.toLowerCase();
  if (!extension) return "□";
  if (["ts", "tsx", "js", "jsx", "vue", "json", "jsonc"].includes(extension)) return "{}";
  if (extension === "rs") return "RS";
  if (extension === "toml") return "TO";
  if (extension === "lock") return "LO";
  return extension.slice(0, 2).toUpperCase();
});

const setPathLoading = (path: string, loading: boolean) => {
  const next = new Set(loadingPaths.value);
  if (loading) {
    next.add(path);
  } else {
    next.delete(path);
  }
  loadingPaths.value = Array.from(next);
};

const setPathExpanded = (path: string, expanded: boolean) => {
  const next = new Set(expandedPaths.value);
  if (expanded) {
    next.add(path);
  } else {
    next.delete(path);
  }
  expandedPaths.value = Array.from(next);
};

const loadDirectory = async (path = "") => {
  setPathLoading(path, true);
  try {
    const listing = await listWorkspaceDirectory(props.conversationId ?? null, path);
    if (!path) {
      rootListing.value = listing;
    }
    childrenByPath.value = {
      ...childrenByPath.value,
      [listing.relativePath]: listing.entries,
    };
    rootError.value = "";
  } catch (error) {
    console.error("Failed to load workspace directory:", error);
    rootError.value = String(error);
    emitToast({ variant: "error", source: "workspace", message: "读取工作区目录失败。" });
  } finally {
    setPathLoading(path, false);
  }
};

const applyRootListing = (listing: WorkspaceDirectoryListing) => {
  rootListing.value = listing;
  childrenByPath.value = {
    [listing.relativePath]: listing.entries,
  };
  expandedPaths.value = [];
  selectedFile.value = null;
  selectedContent.value = null;
  previewError.value = "";
  rootError.value = "";
  filterQuery.value = "";
};

const reloadWorkspace = async () => {
  childrenByPath.value = {};
  expandedPaths.value = [];
  selectedFile.value = null;
  selectedContent.value = null;
  previewError.value = "";
  await loadDirectory("");
};

const toggleDirectory = async (entry: WorkspaceEntry) => {
  const isExpanded = expandedPaths.value.includes(entry.relativePath);
  if (isExpanded) {
    setPathExpanded(entry.relativePath, false);
    return;
  }

  setPathExpanded(entry.relativePath, true);
  if (!childrenByPath.value[entry.relativePath]) {
    await loadDirectory(entry.relativePath);
  }
};

const selectFile = async (entry: WorkspaceEntry) => {
  selectedFile.value = entry;
  selectedContent.value = null;
  previewError.value = "";
  isReadingFile.value = true;
  try {
    selectedContent.value = await readWorkspaceTextFile(props.conversationId ?? null, entry.relativePath);
  } catch (error) {
    console.error("Failed to read workspace file:", error);
    previewError.value = String(error);
  } finally {
    isReadingFile.value = false;
  }
};

const openSelectedFile = () => {
  if (!selectedFile.value) {
    emitToast({ variant: "error", source: "workspace", message: "请先从右侧工作区目录树中选择文件。" });
    return;
  }
  void selectFile(selectedFile.value);
};

const changeWorkspaceRoot = async () => {
  if (isChangingWorkspace.value) {
    return;
  }

  try {
    const selectedPath = await openDialog({
      directory: true,
      multiple: false,
      title: "选择工作区",
    });
    const path = Array.isArray(selectedPath) ? selectedPath[0] : selectedPath;
    if (!path || typeof path !== "string") {
      return;
    }

    isChangingWorkspace.value = true;
    const listing = await setWorkspaceRoot(props.conversationId ?? null, path);
    applyRootListing(listing);
    isFileTreeVisible.value = true;
    emitToast({ variant: "success", source: "workspace", message: "工作区已切换。" });
  } catch (error) {
    console.error("Failed to change workspace root:", error);
    emitToast({ variant: "error", source: "workspace", message: "更换工作区失败。" });
  } finally {
    isChangingWorkspace.value = false;
  }
};

const closeMoreMenu = () => {
  isMoreMenuOpen.value = false;
};

const toggleMoreMenu = () => {
  isMoreMenuOpen.value = !isMoreMenuOpen.value;
};

const copyCurrentPath = async () => {
  const pathToCopy = selectedContent.value?.path || selectedFile.value?.path || rootListing.value?.root;
  closeMoreMenu();
  if (!pathToCopy) {
    emitToast({ variant: "error", source: "workspace", message: "当前没有可复制的路径。" });
    return;
  }

  try {
    await navigator.clipboard.writeText(pathToCopy);
    emitToast({ variant: "success", source: "workspace", message: "路径已复制。" });
  } catch (error) {
    console.error("Failed to copy workspace path:", error);
    emitToast({ variant: "error", source: "workspace", message: "复制路径失败。" });
  }
};

const togglePreviewWrap = () => {
  isPreviewWrapEnabled.value = !isPreviewWrapEnabled.value;
  closeMoreMenu();
};

const onDocumentMouseDown = (event: MouseEvent) => {
  if (!isMoreMenuOpen.value) {
    return;
  }
  const target = event.target as Node | null;
  if (target && moreMenuRef.value?.contains(target)) {
    return;
  }
  closeMoreMenu();
};

const onWindowKeyDown = (event: KeyboardEvent) => {
  if (event.key === "Escape") {
    closeMoreMenu();
  }
};

const clampFileTreeWidth = (width: number) => {
  const containerWidth = workspaceBodyRef.value?.clientWidth ?? 760;
  const maxByContainer = Math.max(FILE_TREE_MIN_WIDTH, containerWidth - PREVIEW_MIN_WIDTH);
  const maxWidth = Math.min(FILE_TREE_MAX_WIDTH, maxByContainer);
  return Math.min(Math.max(width, FILE_TREE_MIN_WIDTH), maxWidth);
};

const stopFileTreeResize = () => {
  if (!isResizingFileTree.value) {
    return;
  }

  isResizingFileTree.value = false;
  window.removeEventListener("pointermove", handleFileTreeResizeMove);
  window.removeEventListener("pointerup", stopFileTreeResize);
  window.removeEventListener("pointercancel", stopFileTreeResize);
  document.body.style.cursor = previousBodyCursor;
  document.body.style.userSelect = previousBodyUserSelect;
};

const handleFileTreeResizeMove = (event: PointerEvent) => {
  if (!isResizingFileTree.value) {
    return;
  }

  event.preventDefault();
  const deltaX = event.clientX - resizeStartX;
  fileTreeWidth.value = clampFileTreeWidth(resizeStartWidth - deltaX);
};

const startFileTreeResize = (event: PointerEvent) => {
  event.preventDefault();
  resizeStartX = event.clientX;
  resizeStartWidth = fileTreeWidth.value;
  isResizingFileTree.value = true;
  previousBodyCursor = document.body.style.cursor;
  previousBodyUserSelect = document.body.style.userSelect;
  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";
  window.addEventListener("pointermove", handleFileTreeResizeMove, { passive: false });
  window.addEventListener("pointerup", stopFileTreeResize);
  window.addEventListener("pointercancel", stopFileTreeResize);
};

const toggleFileTree = () => {
  if (isResizingFileTree.value) {
    stopFileTreeResize();
  }
  isFileTreeVisible.value = !isFileTreeVisible.value;
};

onMounted(() => {
  document.addEventListener("mousedown", onDocumentMouseDown);
  window.addEventListener("keydown", onWindowKeyDown);
});

watch(
  () => props.conversationId,
  () => {
    childrenByPath.value = {};
    expandedPaths.value = [];
    selectedFile.value = null;
    selectedContent.value = null;
    previewError.value = "";
    void loadDirectory("");
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  stopFileTreeResize();
  document.removeEventListener("mousedown", onDocumentMouseDown);
  window.removeEventListener("keydown", onWindowKeyDown);
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-white text-[#202124] dark:bg-[#1e1e1e] dark:text-[#ececec]">
    <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 dark:border-[#333]">
      <div class="flex min-w-0 items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          class="h-8 max-w-[200px] justify-start gap-1.5 rounded-lg bg-[#f7f7f8] px-2.5 text-[13px] font-normal text-[#202124] hover:bg-[#f1f3f4] dark:bg-[#2b2b2b] dark:text-[#ececec] dark:hover:bg-[#343434]"
          @click="openSelectedFile"
        >
          <span class="shrink-0 text-[11px] font-semibold text-[#e66a1a]">{{ selectedFile ? fileIconLabel : "□" }}</span>
          <span class="truncate">{{ selectedFile?.name || "打开文件" }}</span>
        </Button>
        <Button type="button" variant="ghost" size="icon-sm" class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]" @click="reloadWorkspace">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
            <path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
          </svg>
        </Button>
      </div>

      <div class="flex items-center gap-2">
        <div ref="moreMenuRef" class="relative">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]"
            :class="isMoreMenuOpen ? 'bg-[#f7f7f8] dark:bg-[#2d2d2d]' : ''"
            title="更多"
            @click.stop="toggleMoreMenu"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
              <circle cx="5" cy="12" r="1.6" fill="currentColor" />
              <circle cx="12" cy="12" r="1.6" fill="currentColor" />
              <circle cx="19" cy="12" r="1.6" fill="currentColor" />
            </svg>
          </Button>

          <div
            v-if="isMoreMenuOpen"
            class="absolute right-0 top-9 z-30 w-52 rounded-xl border border-[#e5e7eb] bg-white p-1 shadow-[0_14px_40px_rgba(15,23,42,0.14)] dark:border-[#333] dark:bg-[#252525]"
            @click.stop
          >
            <button
              type="button"
              class="flex h-9 w-full items-center gap-3 rounded-lg px-3 text-left text-sm text-[#202124] hover:bg-[#f3f4f6] dark:text-[#ececec] dark:hover:bg-[#303030]"
              @click="copyCurrentPath"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
                <rect x="8" y="8" width="11" height="11" rx="2" stroke="currentColor" stroke-width="1.8" />
                <rect x="4" y="4" width="11" height="11" rx="2" stroke="currentColor" stroke-width="1.8" />
              </svg>
              <span>复制路径</span>
            </button>
            <button
              type="button"
              class="flex h-9 w-full items-center gap-3 rounded-lg px-3 text-left text-sm text-[#202124] hover:bg-[#f3f4f6] dark:text-[#ececec] dark:hover:bg-[#303030]"
              :class="isPreviewWrapEnabled ? 'bg-[#f3f4f6] dark:bg-[#303030]' : ''"
              @click="togglePreviewWrap"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
                <path d="M4 7h11a4 4 0 0 1 0 8H8" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                <path d="m10 12-3 3 3 3" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
              <span>{{ isPreviewWrapEnabled ? "关闭自动换行" : "启用自动换行" }}</span>
            </button>
          </div>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] disabled:cursor-wait disabled:opacity-60 dark:hover:bg-[#2d2d2d]"
          :disabled="isChangingWorkspace"
          title="更换工作区"
          @click="changeWorkspaceRoot"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <path d="M3 8a3 3 0 0 1 3-3h4l2 2h6a3 3 0 0 1 3 3v6.5a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5V8Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
            <path d="M15.5 11H19l-1.4-1.4M8.5 15H5l1.4 1.4M18.8 11A4.5 4.5 0 0 0 11 13M5.2 15A4.5 4.5 0 0 0 13 13" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-8 w-8 rounded-lg text-[#202124] hover:bg-[#f1f3f4] dark:text-[#ececec] dark:hover:bg-[#363636]"
          :class="isFileTreeVisible ? 'bg-[#f7f7f8] dark:bg-[#2d2d2d]' : 'bg-transparent'"
          :aria-pressed="isFileTreeVisible"
          :title="isFileTreeVisible ? '隐藏文件列表' : '显示文件列表'"
          @click="toggleFileTree"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
            <path d="M3 7.8A2.8 2.8 0 0 1 5.8 5H10l2 2h6.2A2.8 2.8 0 0 1 21 9.8v6.4a2.8 2.8 0 0 1-2.8 2.8H5.8A2.8 2.8 0 0 1 3 16.2V7.8Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
          </svg>
        </Button>
      </div>
    </div>

    <div ref="workspaceBodyRef" class="flex min-h-0 flex-1">
      <section class="flex min-w-0 flex-1 flex-col">
        <div v-if="selectedFile" class="flex h-9 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 dark:border-[#333]">
          <div class="min-w-0 text-[13px] text-[#6b7280] dark:text-[#aaa]">
            <span class="text-[#6b7280]">Nova</span>
            <span class="px-1.5 text-[#9aa0a6]">›</span>
            <span class="font-semibold text-[#202124] dark:text-[#ececec]">{{ selectedFile.name }}</span>
          </div>
          <div class="flex items-center gap-0.5">
            <Button type="button" variant="ghost" size="icon-sm" class="h-6 w-6 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
                <circle cx="12" cy="12" r="1.5" fill="currentColor" />
                <circle cx="5" cy="12" r="1.5" fill="currentColor" />
                <circle cx="19" cy="12" r="1.5" fill="currentColor" />
              </svg>
            </Button>
            <Button type="button" variant="ghost" size="icon-sm" class="h-6 w-6 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
                <path d="M14 3h7v7M10 14 21 3M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </Button>
          </div>
        </div>

        <div v-if="!selectedFile" class="flex h-full flex-col items-center justify-center px-6 text-center">
          <svg width="42" height="42" viewBox="0 0 24 24" fill="none" class="text-[#70757a]">
            <path d="M3 7.8A2.8 2.8 0 0 1 5.8 5H10l2 2h6.2A2.8 2.8 0 0 1 21 9.8v6.4a2.8 2.8 0 0 1-2.8 2.8H5.8A2.8 2.8 0 0 1 3 16.2V7.8Z" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
          </svg>
          <div class="mt-4 text-lg font-semibold text-[#111827] dark:text-[#ececec]">打开文件</div>
          <p class="mt-2 text-sm text-[#64748b] dark:text-[#aaa]">从工作区目录树中选择文件</p>
        </div>

        <div v-else class="min-h-0 flex-1 overflow-auto">
          <div v-if="isReadingFile" class="m-4 rounded-lg border border-[#e5e7eb] bg-[#f9fafb] px-3 py-2 text-sm text-[#6b7280] dark:border-[#333] dark:bg-[#252525] dark:text-[#aaa]">
            正在读取文件...
          </div>
          <div v-else-if="previewError" class="m-4 rounded-lg border border-[#fecaca] bg-[#fef2f2] px-3 py-2 text-sm text-[#b91c1c] dark:border-[#4a2424] dark:bg-[#2b1d1d] dark:text-[#fca5a5]">
            {{ previewError }}
          </div>
          <div v-else-if="selectedContent" class="grid grid-cols-[48px_minmax(0,1fr)] py-2 font-mono text-[12px] leading-6" :class="previewCodeGridClass">
            <template v-for="(line, index) in selectedPreviewLines" :key="index">
              <div class="select-none pr-3 text-right text-[#6b7280]">{{ index + 1 }}</div>
              <pre
                class="min-h-6 pr-6 text-[#202124] transition-colors dark:text-[#ececec]"
                :class="previewCodeLineClass"
                v-html="line"
              />
            </template>
          </div>
        </div>
      </section>

      <button
        type="button"
        class="group flex shrink-0 cursor-col-resize items-stretch justify-center overflow-hidden outline-none transition-all duration-200 ease-out"
        :class="[
          isFileTreeVisible ? 'w-2 opacity-100' : 'pointer-events-none w-0 opacity-0',
          isResizingFileTree ? 'bg-[#e8f0fe] dark:bg-[#1f2f46]' : '',
        ]"
        aria-label="调整文件列表宽度"
        title="拖拽调整文件列表宽度"
        @pointerdown="startFileTreeResize"
      >
        <span class="w-px bg-[#e5e7eb] transition-colors group-hover:bg-[#1a73e8] dark:bg-[#333]" />
      </button>

      <aside
        class="flex min-w-0 shrink-0 flex-col overflow-hidden py-3 transition-all duration-200 ease-out"
        :class="isFileTreeVisible ? 'px-2 opacity-100' : 'pointer-events-none px-0 opacity-0'"
        :aria-hidden="!isFileTreeVisible"
        :style="fileTreePaneStyle"
      >
        <div class="relative">
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 text-[#9aa0a6]" width="15" height="15" viewBox="0 0 24 24" fill="none">
            <circle cx="11" cy="11" r="7" stroke="currentColor" stroke-width="1.9" />
            <path d="m16.5 16.5 4 4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
          </svg>
          <Input
            v-model="filterQuery"
            type="search"
            placeholder="筛选文件..."
            class="h-9 rounded-xl border-[#e5e7eb] bg-white pl-9 text-sm text-[#202124] shadow-none focus-visible:ring-0 dark:border-[#333] dark:bg-[#252525] dark:text-[#ececec]"
          />
        </div>

        <div v-if="rootError" class="rounded-lg border border-[#fecaca] bg-[#fef2f2] px-3 py-2 text-sm text-[#b91c1c] dark:border-[#4a2424] dark:bg-[#2b1d1d] dark:text-[#fca5a5]">
          {{ rootError }}
        </div>
        <div v-else class="mt-2 min-h-0 flex-1 overflow-y-auto pr-1">
          <WorkspaceFileTreeNode
            v-for="entry in visibleRootEntries"
            :key="entry.relativePath"
            :entry="entry"
            :depth="0"
            :expandedPaths="expandedPaths"
            :loadingPaths="loadingPaths"
            :selectedPath="selectedFile?.relativePath"
            :childrenByPath="childrenByPath"
            :filterQuery="filterQuery"
            @toggle-directory="toggleDirectory"
            @select-file="selectFile"
          />
        </div>
      </aside>
    </div>
  </div>
</template>
