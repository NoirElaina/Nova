<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { emitToast } from "../../../lib/toast";
import {
  listWorkspaceDirectory,
  type WorkspaceDirectoryListing,
  type WorkspaceEntry,
} from "../../../features/workspace/workspace-api";
import WorkspaceFileTreeNode from "./WorkspaceFileTreeNode.vue";

const props = defineProps<{
  conversationId?: string | null;
}>();

// 工作区页只展示一列文件夹目录树：目录列表 + 筛选，无文件预览区。
const rootListing = ref<WorkspaceDirectoryListing | null>(null);
const childrenByPath = ref<Record<string, WorkspaceEntry[]>>({});
const expandedPaths = ref<string[]>([]);
const loadingPaths = ref<string[]>([]);
const filterQuery = ref("");
const selectedFile = ref<WorkspaceEntry | null>(null);
const rootError = ref("");
const moreMenuRef = ref<HTMLElement | null>(null);
const isMoreMenuOpen = ref(false);

const rootEntries = computed(() => childrenByPath.value[""] ?? []);

const normalizedFilter = computed(() => filterQuery.value.trim().toLowerCase());

const entryMatchesFilter = (entry: WorkspaceEntry): boolean => {
  if (!normalizedFilter.value) return true;
  if (entry.name.toLowerCase().includes(normalizedFilter.value)) return true;
  const children = childrenByPath.value[entry.relativePath] ?? [];
  return children.some(entryMatchesFilter);
};

const visibleRootEntries = computed(() => rootEntries.value.filter(entryMatchesFilter));

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

const reloadWorkspace = async () => {
  childrenByPath.value = {};
  expandedPaths.value = [];
  selectedFile.value = null;
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

// 仅记录选中文件用于高亮与路径复制，不再加载预览内容。
const selectFile = (entry: WorkspaceEntry) => {
  selectedFile.value = entry;
};

const closeMoreMenu = () => {
  isMoreMenuOpen.value = false;
};

const toggleMoreMenu = () => {
  isMoreMenuOpen.value = !isMoreMenuOpen.value;
};

const copyCurrentPath = async () => {
  const pathToCopy = selectedFile.value?.path || rootListing.value?.root;
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
    void loadDirectory("");
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDocumentMouseDown);
  window.removeEventListener("keydown", onWindowKeyDown);
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-white text-[#202124] dark:bg-[#1e1e1e] dark:text-[#ececec]">
    <!-- 工具行：刷新 / 更多 -->
    <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-3 dark:border-[#333]">
      <div class="min-w-0 text-[13px] font-medium text-[#202124] dark:text-[#ececec]">工作区目录</div>

      <div class="flex items-center gap-1">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          class="h-7 w-7 rounded-md text-[#6b7280] hover:bg-[#f7f7f8] dark:hover:bg-[#2d2d2d]"
          title="刷新目录"
          @click="reloadWorkspace"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12a9 9 0 1 1-2.64-6.36"/>
            <path d="M21 3v6h-6"/>
          </svg>
        </Button>

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
          </div>
        </div>
      </div>
    </div>

    <!-- 单列内容：筛选框 + 目录树 -->
    <div class="flex min-h-0 flex-1 flex-col px-2 py-3">
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

      <div v-if="rootError" class="mt-2 rounded-lg border border-[#fecaca] bg-[#fef2f2] px-3 py-2 text-sm text-[#b91c1c] dark:border-[#4a2424] dark:bg-[#2b1d1d] dark:text-[#fca5a5]">
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
    </div>
  </div>
</template>
