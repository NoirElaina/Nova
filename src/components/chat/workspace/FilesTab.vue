<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { readRagDocument, type RagDocumentContent, type RagDocumentMeta } from '../../../features/chat/services/chat-api';

const props = defineProps<{
  files: RagDocumentMeta[];
  selectedFileId?: string | null;
}>();

const selectedId = ref<string | null>(null);
const selectedDocument = ref<RagDocumentContent | null>(null);
const loadingId = ref<string | null>(null);
const errorMessage = ref('');

const selectedMeta = computed(() => props.files.find((file) => file.id === selectedId.value) ?? null);

const formatDocTime = (ts: number) => {
  const date = new Date(ts * 1000);
  if (Number.isNaN(date.getTime())) {
    return '--';
  }
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
};

const formatChars = (count?: number) => {
  if (!Number.isFinite(count ?? 0) || !count) {
    return '0 字符';
  }
  return `${count.toLocaleString()} 字符`;
};

const selectFile = async (file: RagDocumentMeta) => {
  selectedId.value = file.id;
  selectedDocument.value = null;
  errorMessage.value = '';
  loadingId.value = file.id;

  try {
    const document = await readRagDocument(file.id);
    if (selectedId.value !== file.id) {
      return;
    }
    selectedDocument.value = document;
    if (!document) {
      errorMessage.value = '没有找到这个文件内容，可能已经被删除。';
    }
  } catch (error) {
    if (selectedId.value === file.id) {
      errorMessage.value = `读取文件失败：${String(error)}`;
    }
  } finally {
    if (loadingId.value === file.id) {
      loadingId.value = null;
    }
  }
};

watch(
  () => props.files,
  (files) => {
    if (!files.length) {
      selectedId.value = null;
      selectedDocument.value = null;
      return;
    }
    if (!selectedId.value || !files.some((file) => file.id === selectedId.value)) {
      void selectFile(files[0]);
    }
  },
  { immediate: true },
);

watch(
  () => props.selectedFileId,
  (fileId) => {
    if (!fileId || selectedId.value === fileId) {
      return;
    }
    const file = props.files.find((item) => item.id === fileId);
    if (file) {
      void selectFile(file);
    } else {
      selectedId.value = fileId;
      selectedDocument.value = null;
      errorMessage.value = '没有找到这个文件，可能文件列表还没有刷新。';
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="flex h-full min-h-0 bg-[#faf9f6] dark:bg-[#1e1e1e]">
    <aside class="flex w-[320px] shrink-0 flex-col border-r border-[#e7e2d7] bg-[#f5f2eb] dark:border-[#333] dark:bg-[#202020]">
      <div class="border-b border-[#e7e2d7] px-4 py-3 dark:border-[#333]">
        <div class="text-sm font-semibold text-[#2b261f] dark:text-[#f0ece5]">会话文件</div>
        <div class="mt-1 text-xs text-[#8b8172] dark:text-[#9f978d]">
          {{ files.length }} 个已上传文件
        </div>
      </div>

      <div v-if="files.length === 0" class="px-4 py-8 text-sm leading-6 text-[#8b8172] dark:text-[#a49c92]">
        当前会话还没有已入库文件。你在聊天输入框上传文本文件并发送后，会出现在这里。
      </div>

      <div v-else class="min-h-0 flex-1 overflow-y-auto p-3">
        <button
          v-for="file in files"
          :key="file.id"
          type="button"
          class="mb-2 w-full rounded-xl border px-3 py-2.5 text-left transition-colors"
          :class="selectedId === file.id
            ? 'border-[#d9b28a] bg-white text-[#2b261f] shadow-sm dark:border-[#6d5540] dark:bg-[#2b2925] dark:text-[#f0ece5]'
            : 'border-[#e8e0d4] bg-[#fffdfa] text-[#5e5549] hover:bg-white dark:border-[#353535] dark:bg-[#262626] dark:text-[#d5cec3] dark:hover:bg-[#2c2c2c]'"
          @click="selectFile(file)"
        >
          <div class="flex items-start gap-2">
            <span class="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-current/15 bg-current/5">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                <polyline points="14 2 14 8 20 8" />
              </svg>
            </span>
            <span class="min-w-0 flex-1">
              <span class="block truncate text-sm font-medium" :title="file.sourceName">{{ file.sourceName }}</span>
              <span class="mt-1 block text-xs opacity-70">{{ formatChars(file.contentChars) }}</span>
            </span>
          </div>
          <div class="mt-2 line-clamp-2 text-xs leading-5 opacity-75">
            {{ file.preview }}
          </div>
        </button>
      </div>
    </aside>

    <section class="flex min-w-0 flex-1 flex-col">
      <div class="border-b border-[#e7e2d7] px-5 py-4 dark:border-[#333]">
        <template v-if="selectedMeta">
          <div class="flex items-start justify-between gap-4">
            <div class="min-w-0">
              <div class="truncate text-base font-semibold text-[#242019] dark:text-[#f3eee7]" :title="selectedMeta.sourceName">
                {{ selectedMeta.sourceName }}
              </div>
              <div class="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-xs text-[#8a8174] dark:text-[#a49c92]">
                <span>{{ formatChars(selectedMeta.contentChars) }}</span>
                <span v-if="selectedMeta.mimeType">{{ selectedMeta.mimeType }}</span>
                <span>更新于 {{ formatDocTime(selectedMeta.updatedAt) }}</span>
              </div>
            </div>
          </div>
        </template>
        <template v-else>
          <div class="text-base font-semibold text-[#242019] dark:text-[#f3eee7]">文件内容</div>
          <div class="mt-1 text-xs text-[#8a8174] dark:text-[#a49c92]">选择一个文件查看内容。</div>
        </template>
      </div>

      <div class="min-h-0 flex-1 overflow-auto p-5">
        <div v-if="loadingId" class="rounded-2xl border border-[#e7e2d7] bg-white px-4 py-3 text-sm text-[#7b7163] dark:border-[#333] dark:bg-[#252525] dark:text-[#bdb5aa]">
          正在读取文件内容...
        </div>

        <div v-else-if="errorMessage" class="rounded-2xl border border-[#efd0c5] bg-[#fff5f1] px-4 py-3 text-sm text-[#a0563c] dark:border-[#5b342b] dark:bg-[#2f211d] dark:text-[#e7a08a]">
          {{ errorMessage }}
        </div>

        <pre
          v-else-if="selectedDocument"
          class="min-h-full whitespace-pre-wrap break-words rounded-2xl border border-[#e7e2d7] bg-[#fffefa] p-4 font-mono text-[12px] leading-6 text-[#2d2922] shadow-sm dark:border-[#333] dark:bg-[#242424] dark:text-[#e5ded4]"
        >{{ selectedDocument.content }}</pre>

        <div v-else class="rounded-2xl border border-dashed border-[#d8d0c2] px-4 py-10 text-center text-sm text-[#8b8172] dark:border-[#444] dark:text-[#a49c92]">
          暂无可查看内容。
        </div>
      </div>
    </section>
  </div>
</template>
