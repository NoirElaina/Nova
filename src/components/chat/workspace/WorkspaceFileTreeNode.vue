<script setup lang="ts">
import { computed } from "vue";
import { Button } from "@/components/ui/button";
import type { WorkspaceEntry } from "../../../features/workspace/workspace-api";

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
  (event: "toggle-directory", entry: WorkspaceEntry): void;
  (event: "select-file", entry: WorkspaceEntry): void;
}>();

const isDirectory = computed(() => props.entry.kind === "directory");
const isExpanded = computed(() => props.expandedPaths.includes(props.entry.relativePath));
const isLoading = computed(() => props.loadingPaths.includes(props.entry.relativePath));
const isSelected = computed(() => props.selectedPath === props.entry.relativePath);
const children = computed(() => props.childrenByPath[props.entry.relativePath] ?? []);

const normalizedFilter = computed(() => (props.filterQuery || "").trim().toLowerCase());

const entryMatchesFilter = (entry: WorkspaceEntry): boolean => {
  if (!normalizedFilter.value) return true;
  if (entry.name.toLowerCase().includes(normalizedFilter.value)) return true;
  const childEntries = props.childrenByPath[entry.relativePath] ?? [];
  return childEntries.some(entryMatchesFilter);
};

const visibleChildren = computed(() => children.value.filter(entryMatchesFilter));

const iconLabel = computed(() => {
  if (isDirectory.value) return "";
  const extension = props.entry.extension?.toLowerCase();
  if (!extension) return "·";
  if (["md", "mdx"].includes(extension)) return "M";
  if (["rs"].includes(extension)) return "RS";
  if (["lock"].includes(extension)) return "LO";
  if (["toml"].includes(extension)) return "TO";
  if (["html", "css"].includes(extension)) return "#";
  if (["ts", "tsx", "js", "jsx", "vue", "json", "jsonc"].includes(extension)) return "{}";
  return extension.slice(0, 2).toUpperCase();
});

const handleClick = () => {
  if (isDirectory.value) {
    emit("toggle-directory", props.entry);
    return;
  }
  emit("select-file", props.entry);
};
</script>

<template>
  <div>
    <Button
      type="button"
      variant="ghost"
      class="h-8 w-full justify-start gap-2 rounded-md px-2 py-0 text-left text-[0.86rem] font-normal text-[#202124] hover:bg-[#f1f3f4] dark:text-[#ececec] dark:hover:bg-[#2d2d2d]"
      :class="isSelected ? 'bg-[#f1f3f4] ring-1 ring-[#1a73e8] ring-inset dark:bg-[#2d2d2d]' : ''"
      :style="{ paddingLeft: `${8 + depth * 18}px` }"
      @click="handleClick"
    >
      <span
        class="flex h-4 w-4 shrink-0 items-center justify-center text-[#6f7378] transition-transform"
        :class="isExpanded ? 'rotate-90' : ''"
      >
        <svg v-if="isDirectory" width="14" height="14" viewBox="0 0 24 24" fill="none">
          <path d="m9 18 6-6-6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </span>
      <span class="flex h-5 w-5 shrink-0 items-center justify-center">
        <svg v-if="isDirectory" width="15" height="15" viewBox="0 0 24 24" fill="none" class="text-[#6f7378]">
          <path d="M3 7.8A2.8 2.8 0 0 1 5.8 5H10l2 2h6.2A2.8 2.8 0 0 1 21 9.8v6.4a2.8 2.8 0 0 1-2.8 2.8H5.8A2.8 2.8 0 0 1 3 16.2V7.8Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
        </svg>
        <span v-else class="text-[10px] font-bold leading-none text-[#e66a1a]">{{ iconLabel }}</span>
      </span>
      <span class="min-w-0 flex-1 truncate" :title="entry.relativePath || entry.name">{{ entry.name }}</span>
      <span v-if="isLoading" class="text-xs text-[#9aa0a6]">...</span>
    </Button>

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
