<script setup lang="ts">
import type { PendingUploadFile } from '@/lib/chat-types';

defineProps<{
  files: PendingUploadFile[];
}>();

const emit = defineEmits<{
  (e: 'remove', index: number): void;
}>();

const formatFileSize = (bytes: number) => {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return '0 B';
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const kb = bytes / 1024;
  if (kb < 1024) {
    return `${kb.toFixed(1)} KB`;
  }
  const mb = kb / 1024;
  return `${mb.toFixed(1)} MB`;
};
</script>

<template>
  <div v-if="files.length > 0" class="px-3 pt-3 pb-1">
    <div class="flex flex-wrap gap-2">
      <div
        v-for="(file, index) in files"
        :key="`${file.sourceName}-${index}`"
        class="inline-flex items-center gap-2 rounded-lg border border-[#e5e7eb] dark:border-[#474747] bg-[#f8fafc] dark:bg-[#323232] px-2.5 py-1.5 text-[12px] text-[#475569] dark:text-[#d7d0c5]"
      >
        <svg
          v-if="file.kind === 'image'"
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
          <circle cx="8.5" cy="8.5" r="1.5" />
          <path d="M21 15l-5-5L5 21" />
        </svg>
        <svg
          v-else
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
        </svg>
        <span class="max-w-[160px] truncate" :title="file.sourceName">{{ file.sourceName }}</span>
        <span class="text-[11px] opacity-75">{{ formatFileSize(file.size) }}</span>
        <span
          v-if="file.kind === 'document'"
          class="rounded-md bg-black/5 px-1.5 py-0.5 text-[10px] leading-none text-[#64748b] dark:bg-white/10 dark:text-[#cbd5e1]"
          title="上传的文件将保存为会话文件，AI 可通过 Read 工具随时读取。"
        >
          会话文件
        </span>
        <button
          type="button"
          class="ml-1 rounded p-0.5 hover:bg-black/5 dark:hover:bg-white/10 text-muted-foreground hover:text-foreground"
          title="移除"
          @click="emit('remove', index)"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>
