<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { readSessionFile, type SessionFileMeta } from '../../../features/chat/services/chat-api';

const props = defineProps<{
  files: SessionFileMeta[];
  selectedFileId?: string | null;
}>();

const selectedPath = ref<string | null>(null);
const selectedContent = ref<string | null>(null);
const loadingPath = ref<string | null>(null);
const errorMessage = ref('');

const selectedMeta = computed(() => props.files.find((file) => file.readPath === selectedPath.value) ?? null);

const formatDocTime = (ts: number) => {
  const date = new Date(ts * 1000);
  if (Number.isNaN(date.getTime())) {
    return '--';
  }
  return date.toLocaleString("zh-CN", {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
};

const formatFileSize = (bytes: number) => {
  if (!Number.isFinite(bytes) || !bytes) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
};

const selectFile = async (file: SessionFileMeta) => {
  selectedPath.value = file.readPath;
  selectedContent.value = null;
  errorMessage.value = '';
  loadingPath.value = file.readPath;

  try {
    const content = await readSessionFile(file.readPath);
    if (selectedPath.value !== file.readPath) {
      return;
    }
    selectedContent.value = content;
    if (!content) {
      errorMessage.value = '文件内容为空。';
    }
  } catch (error) {
    if (selectedPath.value === file.readPath) {
      errorMessage.value = `读取文件失败：${String(error)}`;
    }
  } finally {
    if (loadingPath.value === file.readPath) {
      loadingPath.value = null;
    }
  }
};

watch(
  () => props.files,
  (files) => {
    if (!files.length) {
      selectedPath.value = null;
      selectedContent.value = null;
      return;
    }
    if (!selectedPath.value || !files.some((file) => file.readPath === selectedPath.value)) {
      void selectFile(files[0]);
    }
  },
  { immediate: true },
);

watch(
  () => props.selectedFileId,
  (readPath) => {
    if (!readPath || selectedPath.value === readPath) {
      return;
    }
    const file = props.files.find((item) => item.readPath === readPath);
    if (file) {
      void selectFile(file);
    } else {
      selectedPath.value = readPath;
      selectedContent.value = null;
      errorMessage.value = '没有找到这个文件，可能文件列表还没有刷新。';
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="flex h-full min-h-0 bg-white text-[#202124] dark:bg-[#1e1e1e] dark:text-[#ececec]">
    <aside class="flex w-[280px] shrink-0 flex-col border-r border-[#e5e7eb] bg-[#fbfbfc] dark:border-[#333] dark:bg-[#1f1f1f]">
      <div class="shrink-0 border-b border-[#e5e7eb] p-2 dark:border-[#333]">
        <div class="flex min-h-10 items-center justify-between rounded-xl border border-[#e7ebf0] bg-white px-3 py-2 shadow-[0_1px_2px_rgba(15,23,42,0.035)] dark:border-[#333] dark:bg-[#242424]">
          <div class="min-w-0">
            <div class="text-[13px] font-medium text-[#202124] dark:text-[#ececec]">会话文件</div>
            <div class="text-[11px] text-[#6b7280] dark:text-[#aaa]">{{ files.length }} 个文件</div>
          </div>
        </div>
      </div>

      <div v-if="files.length === 0" class="px-3 py-4 text-[13px] leading-6 text-[#6b7280] dark:text-[#aaa]">
        当前会话还没有会话文件。上传文件并发送后会出现在这里。
      </div>

      <div v-else class="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        <button
          v-for="file in files"
          :key="file.readPath"
          type="button"
          class="flex w-full items-start gap-2 rounded-md px-2 py-2 text-left transition-colors"
          :class="selectedPath === file.readPath
            ? 'bg-[#f7f7f8] text-[#111827] ring-1 ring-[#1a73e8] ring-inset dark:bg-[#2d2d2d] dark:text-[#ececec]'
            : 'text-[#374151] hover:bg-[#f7f7f8] dark:text-[#d7d7d7] dark:hover:bg-[#2a2a2a]'"
          @click="selectFile(file)"
        >
          <span class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center text-[#6b7280]">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z" />
              <path d="M14 2v5h5" />
            </svg>
          </span>
          <span class="min-w-0 flex-1">
            <span class="block truncate text-[13px] font-medium" :title="file.readPath">{{ file.filename }}</span>
            <span class="mt-0.5 block truncate text-[11px] text-[#6b7280] dark:text-[#aaa]">
              {{ formatFileSize(file.size) }}
            </span>
          </span>
        </button>
      </div>
    </aside>

    <section class="flex min-w-0 flex-1 flex-col bg-white dark:bg-[#1e1e1e]">
      <div class="shrink-0 border-b border-[#e5e7eb] p-2 dark:border-[#333]">
        <div class="flex min-h-10 items-center rounded-xl border border-[#e7ebf0] bg-white px-3 py-2 shadow-[0_1px_2px_rgba(15,23,42,0.035)] dark:border-[#333] dark:bg-[#242424]">
          <template v-if="selectedMeta">
            <div class="min-w-0">
              <div class="truncate text-[13px] font-semibold text-[#202124] dark:text-[#ececec]" :title="selectedMeta.readPath">
                {{ selectedMeta.filename }}
              </div>
              <div class="mt-0.5 flex min-w-0 gap-3 truncate text-[11px] text-[#6b7280] dark:text-[#aaa]">
                <span>{{ formatFileSize(selectedMeta.size) }}</span>
                <span class="truncate">创建于 {{ formatDocTime(selectedMeta.createdAt) }}</span>
              </div>
            </div>
          </template>
          <template v-else>
            <div class="min-w-0">
              <div class="text-[13px] font-semibold text-[#202124] dark:text-[#ececec]">文件内容</div>
              <div class="mt-0.5 text-[11px] text-[#6b7280] dark:text-[#aaa]">选择一个文件查看内容。</div>
            </div>
          </template>
        </div>
      </div>

      <div class="min-h-0 flex-1 overflow-auto">
        <div v-if="loadingPath" class="m-3 rounded-lg border border-[#e5e7eb] bg-[#f9fafb] px-3 py-2 text-[13px] text-[#6b7280] dark:border-[#333] dark:bg-[#252525] dark:text-[#aaa]">
          正在读取文件内容...
        </div>

        <div v-else-if="errorMessage" class="m-3 rounded-lg border border-[#fecaca] bg-[#fef2f2] px-3 py-2 text-[13px] text-[#b91c1c] dark:border-[#4a2424] dark:bg-[#2b1d1d] dark:text-[#fca5a5]">
          {{ errorMessage }}
        </div>

        <pre
          v-else-if="selectedContent"
          class="min-h-full whitespace-pre-wrap break-words p-3 font-mono text-[12px] leading-6 text-[#202124] dark:text-[#ececec]"
        >{{ selectedContent }}</pre>

        <div v-else class="m-3 rounded-lg border border-dashed border-[#d1d5db] px-4 py-8 text-center text-[13px] text-[#6b7280] dark:border-[#444] dark:text-[#aaa]">
          暂无可查看内容。
        </div>
      </div>
    </section>
  </div>
</template>
