<script setup lang="ts">
import { computed } from 'vue';
import type { WorkspaceEntry } from '../../../features/workspace/workspace-api';

const props = defineProps<{
  entry: WorkspaceEntry;
  depth: number;
  expandedPaths: string[];
  loadingPaths: string[];
  selectedPath?: string | null;
  childrenByPath: Record<string, WorkspaceEntry[]>;
  filterQuery?: string;
}>();

const emit = defineEmits<{
  (event: 'toggle-directory', entry: WorkspaceEntry): void;
  (event: 'select-file', entry: WorkspaceEntry): void;
}>();

const isDirectory = computed(() => props.entry.kind === 'directory');
const isExpanded = computed(() => props.expandedPaths.includes(props.entry.relativePath));
const isLoading = computed(() => props.loadingPaths.includes(props.entry.relativePath));
const isSelected = computed(() => props.selectedPath === props.entry.relativePath);
const children = computed(() => props.childrenByPath[props.entry.relativePath] ?? []);

const normalizedFilter = computed(() => (props.filterQuery || '').trim().toLowerCase());

const entryMatchesFilter = (entry: WorkspaceEntry): boolean => {
  if (!normalizedFilter.value) return true;
  if (entry.name.toLowerCase().includes(normalizedFilter.value)) return true;
  const childEntries = props.childrenByPath[entry.relativePath] ?? [];
  return childEntries.some(entryMatchesFilter);
};

const visibleChildren = computed(() => children.value.filter(entryMatchesFilter));

const iconLabel = computed(() => {
  if (isDirectory.value) return '';
  const extension = props.entry.extension?.toLowerCase();
  if (!extension) return '·';
  if (['md', 'mdx'].includes(extension)) return 'M';
  if (['ts', 'tsx', 'js', 'jsx', 'vue'].includes(extension)) return '{}';
  if (['json', 'jsonc'].includes(extension)) return '{}';
  if (['html', 'css'].includes(extension)) return '#';
  return extension.slice(0, 2).toUpperCase();
});

const handleClick = () => {
  if (isDirectory.value) {
    emit('toggle-directory', props.entry);
    return;
  }
  emit('select-file', props.entry);
};
</script>

<template>
  <div>
    <button
      type="button"
      class="workspace-tree-row"
      :class="{ 'workspace-tree-row-selected': isSelected }"
      :style="{ paddingLeft: `${10 + depth * 18}px` }"
      @click="handleClick"
    >
      <span class="workspace-tree-chevron" :class="{ 'workspace-tree-chevron-open': isExpanded }">
        <svg v-if="isDirectory" width="14" height="14" viewBox="0 0 24 24" fill="none">
          <path d="m9 18 6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </span>
      <span class="workspace-tree-icon" :class="isDirectory ? 'workspace-tree-icon-folder' : 'workspace-tree-icon-file'">
        <svg v-if="isDirectory" width="15" height="15" viewBox="0 0 24 24" fill="none">
          <path d="M3 7.8A2.8 2.8 0 0 1 5.8 5H10l2 2h6.2A2.8 2.8 0 0 1 21 9.8v6.4a2.8 2.8 0 0 1-2.8 2.8H5.8A2.8 2.8 0 0 1 3 16.2V7.8Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
        </svg>
        <span v-else>{{ iconLabel }}</span>
      </span>
      <span class="min-w-0 flex-1 truncate" :title="entry.relativePath || entry.name">{{ entry.name }}</span>
      <span v-if="isLoading" class="workspace-tree-loading">...</span>
    </button>

    <div v-if="isDirectory && isExpanded">
      <WorkspaceFileTreeNode
        v-for="child in visibleChildren"
        :key="child.relativePath"
        :entry="child"
        :depth="depth + 1"
        :expandedPaths="expandedPaths"
        :loadingPaths="loadingPaths"
        :selectedPath="selectedPath"
        :childrenByPath="childrenByPath"
        :filterQuery="filterQuery"
        @toggle-directory="emit('toggle-directory', $event)"
        @select-file="emit('select-file', $event)"
      />
    </div>
  </div>
</template>

<style scoped>
.workspace-tree-row {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 8px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: #25211b;
  cursor: pointer;
  font-size: 14px;
  min-height: 34px;
  padding-bottom: 0;
  padding-right: 10px;
  padding-top: 0;
  text-align: left;
  transition:
    background 140ms ease,
    color 140ms ease;
}

.workspace-tree-row:hover,
.workspace-tree-row-selected {
  background: #f2eee7;
}

.dark .workspace-tree-row {
  color: #eee8df;
}

.dark .workspace-tree-row:hover,
.dark .workspace-tree-row-selected {
  background: #2c2c2c;
}

.workspace-tree-chevron {
  display: inline-flex;
  height: 18px;
  width: 18px;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  color: #7d766b;
  transition: transform 140ms ease;
}

.workspace-tree-chevron-open {
  transform: rotate(90deg);
}

.workspace-tree-icon {
  display: inline-flex;
  height: 20px;
  min-width: 20px;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 800;
}

.workspace-tree-icon-folder {
  color: #746f67;
}

.workspace-tree-icon-file {
  color: #d47735;
}

.workspace-tree-loading {
  color: #9b9286;
  font-size: 12px;
}
</style>
