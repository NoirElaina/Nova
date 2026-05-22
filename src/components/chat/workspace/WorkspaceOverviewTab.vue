<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { emitToast } from '../../../lib/toast';
import {
  listWorkspaceDirectory,
  readWorkspaceTextFile,
  type WorkspaceDirectoryListing,
  type WorkspaceEntry,
  type WorkspaceFileContent,
} from '../../../features/workspace/workspace-api';
import WorkspaceFileTreeNode from './WorkspaceFileTreeNode.vue';

const rootListing = ref<WorkspaceDirectoryListing | null>(null);
const childrenByPath = ref<Record<string, WorkspaceEntry[]>>({});
const expandedPaths = ref<string[]>([]);
const loadingPaths = ref<string[]>([]);
const filterQuery = ref('');
const selectedFile = ref<WorkspaceEntry | null>(null);
const selectedContent = ref<WorkspaceFileContent | null>(null);
const previewError = ref('');
const isReadingFile = ref(false);
const rootError = ref('');

const rootEntries = computed(() => childrenByPath.value[''] ?? []);
const rootPath = computed(() => rootListing.value?.root ?? '/');
const displayPath = computed(() => selectedFile.value?.relativePath || '/');

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

const loadDirectory = async (path = '') => {
  setPathLoading(path, true);
  try {
    const listing = await listWorkspaceDirectory(path);
    if (!path) {
      rootListing.value = listing;
    }
    childrenByPath.value = {
      ...childrenByPath.value,
      [listing.relativePath]: listing.entries,
    };
    rootError.value = '';
  } catch (error) {
    console.error('Failed to load workspace directory:', error);
    rootError.value = String(error);
    emitToast({ variant: 'error', source: 'workspace', message: '读取工作区目录失败。' });
  } finally {
    setPathLoading(path, false);
  }
};

const reloadWorkspace = async () => {
  childrenByPath.value = {};
  expandedPaths.value = [];
  selectedFile.value = null;
  selectedContent.value = null;
  previewError.value = '';
  await loadDirectory('');
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
  previewError.value = '';
  isReadingFile.value = true;
  try {
    selectedContent.value = await readWorkspaceTextFile(entry.relativePath);
  } catch (error) {
    console.error('Failed to read workspace file:', error);
    previewError.value = String(error);
  } finally {
    isReadingFile.value = false;
  }
};

const openSelectedFile = () => {
  if (!selectedFile.value) {
    emitToast({ variant: 'error', source: 'workspace', message: '请先从右侧工作区目录树中选择文件。' });
    return;
  }
  void selectFile(selectedFile.value);
};

onMounted(() => {
  void loadDirectory('');
});
</script>

<template>
  <div class="workspace-shell flex h-full min-h-0 flex-col bg-[#fbfaf7] text-[#1f1a13] dark:bg-[#1e1e1e] dark:text-[#f1eee8]">
    <div class="workspace-toolbar">
      <div class="flex items-center gap-2">
        <button type="button" class="workspace-primary-button" @click="openSelectedFile">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
            <path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7l-5-5Z" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round" />
            <path d="M14 2v5h5" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round" />
          </svg>
          打开文件
        </button>
        <button type="button" class="workspace-icon-button" title="刷新工作区" @click="reloadWorkspace">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
            <path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
          </svg>
        </button>
      </div>
      <div class="flex items-center gap-2">
        <button type="button" class="workspace-icon-button" title="适配视图">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <path d="M8 3H5a2 2 0 0 0-2 2v3M16 3h3a2 2 0 0 1 2 2v3M8 21H5a2 2 0 0 1-2-2v-3M16 21h3a2 2 0 0 0 2-2v-3" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
          </svg>
        </button>
        <button type="button" class="workspace-icon-button" title="收起预览">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none">
            <rect x="5" y="7" width="14" height="10" rx="2" stroke="currentColor" stroke-width="1.9" />
            <path d="M9 12h6" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
          </svg>
        </button>
        <button type="button" class="workspace-split-button" title="工作区">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
            <rect x="3" y="4" width="18" height="16" rx="4" stroke="currentColor" stroke-width="1.9" />
            <path d="M14 4v16" stroke="currentColor" stroke-width="1.9" />
          </svg>
        </button>
      </div>
    </div>

    <div class="workspace-pathbar">
      <span class="workspace-current-path">{{ displayPath }}</span>
      <button type="button" class="workspace-floating-folder" title="当前工作区">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
          <path d="M3 7.8A2.8 2.8 0 0 1 5.8 5H10l2 2h6.2A2.8 2.8 0 0 1 21 9.8v6.4a2.8 2.8 0 0 1-2.8 2.8H5.8A2.8 2.8 0 0 1 3 16.2V7.8Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
        </svg>
      </button>
    </div>

    <div class="flex min-h-0 flex-1">
      <section class="workspace-preview-pane">
        <div v-if="!selectedFile" class="workspace-empty-preview">
          <div class="workspace-empty-icon">
            <svg width="36" height="36" viewBox="0 0 24 24" fill="none">
              <path d="M3 7.8A2.8 2.8 0 0 1 5.8 5H10l2 2h6.2A2.8 2.8 0 0 1 21 9.8v6.4a2.8 2.8 0 0 1-2.8 2.8H5.8A2.8 2.8 0 0 1 3 16.2V7.8Z" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
            </svg>
          </div>
          <div class="workspace-empty-title">打开文件</div>
          <p>从工作区目录树中选择文件</p>
        </div>

        <div v-else class="flex h-full min-h-0 flex-col">
          <div class="workspace-preview-header">
            <div class="min-w-0">
              <div class="truncate text-sm font-semibold" :title="selectedFile.relativePath">{{ selectedFile.name }}</div>
              <div class="mt-1 truncate text-xs text-[#8b8172] dark:text-[#aaa197]">{{ selectedFile.relativePath }}</div>
            </div>
          </div>

          <div class="min-h-0 flex-1 overflow-auto p-4">
            <div v-if="isReadingFile" class="workspace-preview-message">正在读取文件...</div>
            <div v-else-if="previewError" class="workspace-preview-message workspace-preview-error">{{ previewError }}</div>
            <pre v-else-if="selectedContent" class="workspace-preview-code">{{ selectedContent.content }}</pre>
          </div>
        </div>
      </section>

      <aside class="workspace-tree-pane">
        <div class="workspace-search-wrap">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <circle cx="11" cy="11" r="7" stroke="currentColor" stroke-width="1.9" />
            <path d="m16.5 16.5 4 4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
          </svg>
          <input v-model="filterQuery" type="search" placeholder="筛选文件..." />
        </div>

        <div class="workspace-root-label" :title="rootPath">{{ rootListing ? '/' : '读取工作区...' }}</div>

        <div v-if="rootError" class="workspace-tree-error">{{ rootError }}</div>
        <div v-else class="workspace-tree-scroll">
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

<style scoped>
.workspace-toolbar {
  display: flex;
  height: 56px;
  flex-shrink: 0;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid #e6e1d8;
  padding: 0 12px;
}

.dark .workspace-toolbar {
  border-color: #333;
}

.workspace-primary-button,
.workspace-icon-button,
.workspace-split-button,
.workspace-floating-folder {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  color: #242019;
  transition:
    background 140ms ease,
    color 140ms ease;
}

.workspace-primary-button {
  gap: 7px;
  height: 34px;
  border-radius: 11px;
  background: #f3f1ed;
  font-size: 14px;
  font-weight: 600;
  padding: 0 13px;
}

.workspace-icon-button {
  height: 34px;
  width: 34px;
  border-radius: 10px;
  background: transparent;
  color: #8b8172;
}

.workspace-split-button,
.workspace-floating-folder {
  height: 38px;
  width: 38px;
  border-radius: 13px;
  background: #f3f1ed;
}

.workspace-primary-button:hover,
.workspace-icon-button:hover,
.workspace-split-button:hover,
.workspace-floating-folder:hover {
  background: #ebe7df;
  color: #15110c;
}

.dark .workspace-primary-button,
.dark .workspace-split-button,
.dark .workspace-floating-folder {
  background: #2c2c2c;
  color: #f1eee8;
}

.dark .workspace-icon-button {
  color: #aaa197;
}

.dark .workspace-primary-button:hover,
.dark .workspace-icon-button:hover,
.dark .workspace-split-button:hover,
.dark .workspace-floating-folder:hover {
  background: #363636;
  color: #fffaf3;
}

.workspace-pathbar {
  position: relative;
  display: flex;
  height: 70px;
  flex-shrink: 0;
  align-items: center;
  border-bottom: 1px solid #e6e1d8;
  padding: 0 16px;
}

.dark .workspace-pathbar {
  border-color: #333;
}

.workspace-current-path {
  min-width: 0;
  overflow: hidden;
  color: #0f172a;
  font-size: 16px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dark .workspace-current-path {
  color: #f1eee8;
}

.workspace-floating-folder {
  position: absolute;
  right: 10px;
  top: 14px;
}

.workspace-preview-pane {
  display: flex;
  min-width: 0;
  width: 42%;
  flex-shrink: 0;
  flex-direction: column;
  border-right: 1px solid #e6e1d8;
}

.dark .workspace-preview-pane {
  border-color: #333;
}

.workspace-empty-preview {
  display: flex;
  height: 100%;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: #5f574c;
  padding: 24px;
  text-align: center;
}

.workspace-empty-icon {
  color: #6f6a62;
}

.workspace-empty-title {
  margin-top: 14px;
  color: #14100b;
  font-size: 18px;
  font-weight: 760;
}

.workspace-empty-preview p {
  margin-top: 9px;
  color: #64748b;
  font-size: 14px;
}

.dark .workspace-empty-preview {
  color: #bbb3a8;
}

.dark .workspace-empty-title {
  color: #fffaf3;
}

.dark .workspace-empty-preview p {
  color: #aaa197;
}

.workspace-preview-header {
  border-bottom: 1px solid #e6e1d8;
  padding: 13px 15px;
}

.dark .workspace-preview-header {
  border-color: #333;
}

.workspace-preview-message {
  border: 1px solid #e7e2d7;
  border-radius: 14px;
  background: #fffdfa;
  color: #746a5c;
  font-size: 13px;
  line-height: 1.7;
  padding: 14px;
}

.workspace-preview-error {
  border-color: #efd0c5;
  background: #fff5f1;
  color: #a0563c;
}

.dark .workspace-preview-message {
  border-color: #383838;
  background: #262626;
  color: #d8d0c2;
}

.dark .workspace-preview-error {
  border-color: #5b342b;
  background: #2f211d;
  color: #e7a08a;
}

.workspace-preview-code {
  min-height: 100%;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  border: 1px solid #e7e2d7;
  border-radius: 16px;
  background: #fffefa;
  color: #27231d;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace;
  font-size: 12px;
  line-height: 1.65;
  padding: 14px;
}

.dark .workspace-preview-code {
  border-color: #383838;
  background: #242424;
  color: #eee6dc;
}

.workspace-tree-pane {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
  padding: 12px 12px 16px;
}

.workspace-search-wrap {
  display: flex;
  height: 36px;
  flex-shrink: 0;
  align-items: center;
  gap: 8px;
  border: 1px solid #e5e0d8;
  border-radius: 12px;
  background: #fff;
  color: #98a1ad;
  padding: 0 11px;
}

.workspace-search-wrap input {
  min-width: 0;
  flex: 1;
  border: 0;
  background: transparent;
  color: #1f1a13;
  font-size: 14px;
  outline: none;
}

.workspace-search-wrap input::placeholder {
  color: #9aa3af;
}

.dark .workspace-search-wrap {
  border-color: #3a3a3a;
  background: #252525;
}

.dark .workspace-search-wrap input {
  color: #f1eee8;
}

.workspace-root-label {
  flex-shrink: 0;
  overflow: hidden;
  padding: 12px 6px 6px;
  color: #0f172a;
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dark .workspace-root-label {
  color: #f1eee8;
}

.workspace-tree-scroll {
  min-height: 0;
  flex: 1;
  overflow-y: auto;
  padding-right: 2px;
}

.workspace-tree-error {
  margin-top: 12px;
  border: 1px solid #efd0c5;
  border-radius: 14px;
  background: #fff5f1;
  color: #a0563c;
  font-size: 13px;
  line-height: 1.7;
  padding: 14px;
}

.dark .workspace-tree-error {
  border-color: #5b342b;
  background: #2f211d;
  color: #e7a08a;
}
</style>
