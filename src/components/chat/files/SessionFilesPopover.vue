<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { Button } from "@/components/ui/button";
import type { PendingUploadFile } from "../../../lib/chat-types";
import SessionFileItem from "./SessionFileItem.vue";

type SessionFileMeta = {
  id: string;
  sourceName: string;
  sourceType: string;
  mimeType?: string;
  contentChars: number;
  preview: string;
  checksum: string;
  createdAt: number;
  updatedAt: number;
};

const props = defineProps<{
  files: SessionFileMeta[];
  pendingUploads: PendingUploadFile[];
}>();

const emit = defineEmits<{
  (e: "open"): void;
  (e: "open-workspace-file", fileId: string): void;
  (e: "remove-pending-upload", index: number): void;
}>();

const rootRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);

const togglePanel = () => {
  const next = !isOpen.value;
  isOpen.value = next;
  if (next) {
    emit("open");
  }
};

const closePanel = () => {
  isOpen.value = false;
};

const onPointerDownDocument = (event: MouseEvent) => {
  if (!isOpen.value || !rootRef.value) {
    return;
  }
  const target = event.target as Node | null;
  if (target && !rootRef.value.contains(target)) {
    closePanel();
  }
};

const formatFileSize = (bytes?: number) => {
  if (!bytes || !Number.isFinite(bytes) || bytes <= 0) {
    return "";
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const kb = bytes / 1024;
  if (kb < 1024) {
    return `${kb.toFixed(1)} KB`;
  }
  return `${(kb / 1024).toFixed(1)} MB`;
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
      class="h-8 px-3 rounded-md border border-[#e5e0d6] dark:border-[#444] bg-white/95 dark:bg-[#262626] text-[12px] text-[#5f584a] dark:text-[#d5cdc0] inline-flex items-center gap-2 hover:bg-[#f7f4ed] dark:hover:bg-[#2f2f2f] transition-colors"
      @click="togglePanel"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <polyline points="14 2 14 8 20 8" />
      </svg>
      会话文件
      <span class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 rounded-full bg-[#efe9dc] dark:bg-[#3a342d] text-[11px] leading-none">
        {{ props.files.length }}
      </span>
    </Button>

    <div
      v-if="isOpen"
      class="absolute right-0 top-10 w-[360px] max-h-[68vh] overflow-hidden rounded-2xl border border-[#e6e1d6] dark:border-[#464646] bg-white dark:bg-[#242424] shadow-[0_18px_56px_rgba(0,0,0,0.18)]"
    >
      <div class="px-3 py-2.5 border-b border-[#eee8dd] dark:border-[#3a3a3a] text-[12px] text-[#726957] dark:text-[#b9b1a6] flex items-center justify-between">
        <span class="font-medium">Artifacts</span>
        <span>{{ props.files.length }} files</span>
      </div>

      <div v-if="props.pendingUploads.length" class="px-3 py-2 border-b border-[#eee8dd] dark:border-[#3a3a3a]">
        <div class="text-[11px] text-[#9a8f7e] mb-2">待发送附件</div>
        <div class="flex flex-wrap gap-1.5">
          <span
            v-for="(file, index) in props.pendingUploads"
            :key="`${file.sourceName}-${index}`"
            class="inline-flex items-center gap-1 rounded-md bg-[#f4f0e8] dark:bg-[#2f2f2f] px-2 py-1 text-[11px] text-[#5e5648] dark:text-[#d2cbc0]"
          >
            <span class="max-w-[150px] truncate" :title="file.sourceName">{{ file.sourceName }}</span>
            <span v-if="formatFileSize(file.size)" class="opacity-70">{{ formatFileSize(file.size) }}</span>
            <Button variant="ghost" size="icon-sm" class="h-4 w-4 p-0 opacity-75 hover:opacity-100" @click="emit('remove-pending-upload', index)">
              ×
            </Button>
          </span>
        </div>
      </div>

      <div v-if="props.files.length === 0" class="px-3 py-5 text-[12px] text-[#9a9283] dark:text-[#9b9489]">
        当前会话还没有已入库文件。
      </div>
      <div v-else class="max-h-[52vh] overflow-y-auto px-2.5 py-2 space-y-2">
        <SessionFileItem
          v-for="doc in props.files"
          :key="doc.id"
          :file="doc"
          class="cursor-pointer"
          @click="emit('open-workspace-file', doc.id)"
        />
      </div>
    </div>
  </div>
</template>
