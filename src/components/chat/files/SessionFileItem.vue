<script setup lang="ts">
import { Card, CardContent } from '@/components/ui/card';

type SessionFileMeta = {
  filename: string;
  readPath: string;
  size: number;
  createdAt: number;
};

const props = defineProps<{
  file: SessionFileMeta;
}>();

const formatDocTime = (ts: number) => {
  const date = new Date(ts * 1000);
  if (Number.isNaN(date.getTime())) {
    return "--";
  }
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
};

const formatFileSize = (bytes: number) => {
  if (!bytes || !Number.isFinite(bytes) || bytes <= 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
};
</script>

<template>
  <Card class="border-[#e5e7eb] bg-[#f8fafc] py-0 transition-colors hover:bg-[#f1f5f9] dark:border-[#3a3a3a] dark:bg-[#2b2b2b] dark:hover:bg-[#333]">
    <CardContent class="px-3 py-2.5">
      <div class="flex items-start justify-between gap-2">
        <div class="flex min-w-0 items-center gap-2">
          <span class="flex h-7 w-7 items-center justify-center rounded-md border border-[#e5e7eb] bg-white text-[#64748b] dark:border-[#4a4a4a] dark:bg-[#222] dark:text-[#c6bfb2]">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
            </svg>
          </span>
          <div class="min-w-0">
            <div class="truncate text-[12px] font-medium text-[#111827] dark:text-[#e2dbcf]" :title="props.file.readPath">
              {{ props.file.filename }}
            </div>
            <div class="text-[10px] text-[#94a3b8] dark:text-[#a79f92]">
              {{ formatFileSize(props.file.size) }}
            </div>
          </div>
        </div>
        <div class="shrink-0 text-[10px] text-[#94a3b8] dark:text-[#9d9589]">
          {{ formatDocTime(props.file.createdAt) }}
        </div>
      </div>
    </CardContent>
  </Card>
</template>
