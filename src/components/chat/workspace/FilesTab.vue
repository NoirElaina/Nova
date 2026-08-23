<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { readSessionFile, type SessionFileMeta } from '../../../features/chat/services/chat-api';
import WorkspaceOverviewTab from './WorkspaceOverviewTab.vue';

const props = defineProps<{
  files: SessionFileMeta[];
  selectedFileId?: string | null;
  conversationId?: string | null;
}>();

/** 子视图：会话文件（上传）/ 工作区目录（项目树），合并前分属两个页签。 */
type FilesView = 'session' | 'workspace';
const activeView = ref<FilesView>('session');

// 用 filename 作为唯一标识（后端不再返回 readPath）
const selectedFilename = ref<string | null>(null);
const selectedContent = ref<string | null>(null);
const loadingFilename = ref<string | null>(null);
const errorMessage = ref('');

const selectedMeta = computed(() =>
  props.files.find((file) => file.filename === selectedFilename.value) ?? null,
);

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
  selectedFilename.value = file.filename;
  selectedContent.value = null;
  errorMessage.value = '';
  loadingFilename.value = file.filename;

  try {
    const convId = props.conversationId;
    if (!convId) {
      errorMessage.value = '无法读取文件：缺少会话 ID';
      return;
    }
    const content = await readSessionFile(convId, file.filename);
    if (selectedFilename.value !== file.filename) {
      return;
    }
    selectedContent.value = content;
    if (!content) {
      errorMessage.value = '文件内容为空。';
    }
  } catch (error) {
    if (selectedFilename.value === file.filename) {
      errorMessage.value = `读取文件失败：${String(error)}`;
    }
  } finally {
    if (loadingFilename.value === file.filename) {
      loadingFilename.value = null;
    }
  }
};

watch(
  () => props.files,
  (files) => {
    if (!files.length) {
      selectedFilename.value = null;
      selectedContent.value = null;
      return;
    }
    if (!selectedFilename.value || !files.some((file) => file.filename === selectedFilename.value)) {
      void selectFile(files[0]);
    }
  },
  { immediate: true },
);

watch(
  () => props.selectedFileId,
  (fileId) => {
    if (!fileId || selectedFilename.value === fileId) {
      return;
    }
    const file = props.files.find((item) => item.filename === fileId);
    if (file) {
      void selectFile(file);
    } else {
      selectedFilename.value = fileId;
      selectedContent.value = null;
      errorMessage.value = '没有找到这个文件，可能文件列表还没有刷新。';
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-white text-[#202124] dark:bg-[#1e1e1e] dark:text-[#ececec]">
    <!-- 子视图切换：会话文件 / 工作区目录 -->
    <div class="flex h-10 shrink-0 items-center gap-0.5 border-b border-[#eef0f3] px-2 dark:border-[#2c2c2c]">
      <button
        type="button"
        class="flex h-7 items-center gap-1.5 rounded-lg px-2 text-[12px] transition-colors"
        :class="activeView === 'session'
          ? 'bg-[#eef2f7] font-medium text-[#111827] dark:bg-white/10 dark:text-[#ececec]'
          : 'text-[#64748b] hover:bg-[#f5f6f8] hover:text-[#334155] dark:text-[#8a8a8a] dark:hover:bg-white/5 dark:hover:text-[#ccc]'"
        @click="activeView = 'session'"
      >
        会话文件
        <span class="rounded-full bg-black/5 px-1.5 py-0.5 text-[10px] text-[#64748b] dark:bg-white/10 dark:text-[#cbd5e1]">{{ files.length }}</span>
      </button>
      <button
        type="button"
        class="flex h-7 items-center gap-1.5 rounded-lg px-2 text-[12px] transition-colors"
        :class="activeView === 'workspace'
          ? 'bg-[#eef2f7] font-medium text-[#111827] dark:bg-white/10 dark:text-[#ececec]'
          : 'text-[#64748b] hover:bg-[#f5f6f8] hover:text-[#334155] dark:text-[#8a8a8a] dark:hover:bg-white/5 dark:hover:text-[#ccc]'"
        @click="activeView = 'workspace'"
      >
        工作区目录
      </button>
    </div>

    <!-- 工作区目录树（独立自含组件，直接嵌入） -->
    <WorkspaceOverviewTab v-if="activeView === 'workspace'" :conversationId="conversationId ?? null" />

    <!-- 会话文件：左列表右预览 -->
    <template v-else>
    <div class="flex min-h-0 flex-1">
    <aside class="flex w-[280px] shrink-0 flex-col border-r border-[#eef0f3] dark:border-[#2c2c2c]">
      <div v-if="files.length === 0" class="px-3 py-4 text-[13px] leading-6 text-[#6b7280] dark:text-[#aaa]">
        当前会话还没有会话文件。上传文件并发送后会出现在这里。
      </div>

      <div v-else class="min-h-0 flex-1 overflow-y-auto px-2 py-1">
        <button
          v-for="file in files"
          :key="file.filename"
          type="button"
          class="flex w-full items-start gap-2 rounded-lg px-2 py-2 text-left transition-colors"
          :class="selectedFilename === file.filename
            ? 'bg-[#eef2f7] text-[#111827] dark:bg-white/10 dark:text-[#ececec]'
            : 'text-[#374151] hover:bg-[#f7f7f8] dark:text-[#d7d7d7] dark:hover:bg-white/5'"
          @click="selectFile(file)"
        >
          <span class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center text-[#6b7280]">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z" />
              <path d="M14 2v5h5" />
            </svg>
          </span>
          <span class="min-w-0 flex-1">
            <span class="block truncate text-[13px] font-medium" :title="file.filename">{{ file.filename }}</span>
            <span class="mt-0.5 block truncate text-[11px] text-[#6b7280] dark:text-[#aaa]">
              {{ formatFileSize(file.size) }}
            </span>
          </span>
        </button>
      </div>
    </aside>

    <section class="flex min-w-0 flex-1 flex-col">
      <!-- 预览头：文件名 + 元信息，平铺不再套卡片 -->
      <div class="flex h-10 shrink-0 items-center border-b border-[#eef0f3] px-3 dark:border-[#2c2c2c]">
        <template v-if="selectedMeta">
          <div class="flex min-w-0 items-baseline gap-2.5">
            <span class="truncate text-[13px] font-medium text-[#202124] dark:text-[#ececec]" :title="selectedMeta.filename">
              {{ selectedMeta.filename }}
            </span>
            <span class="shrink-0 text-[11px] text-[#6b7280] dark:text-[#aaa]">
              {{ formatFileSize(selectedMeta.size) }} · 创建于 {{ formatDocTime(selectedMeta.createdAt) }}
            </span>
          </div>
        </template>
        <span v-else class="text-[13px] text-[#6b7280] dark:text-[#aaa]">选择一个文件查看内容</span>
      </div>

      <div class="min-h-0 flex-1 overflow-auto">
        <div v-if="loadingFilename" class="flex h-full items-center justify-center text-[13px] text-[#6b7280] dark:text-[#aaa]">
          加载中...
        </div>
        <div v-else-if="errorMessage" class="px-4 py-3 text-[13px] text-red-500 dark:text-red-400">
          {{ errorMessage }}
        </div>
        <pre v-else-if="selectedContent" class="whitespace-pre-wrap break-words p-4 font-mono text-[12px] leading-[1.6] text-[#202124] dark:text-[#ececec]">{{ selectedContent }}</pre>
        <div v-else class="flex h-full items-center justify-center text-[13px] text-[#6b7280] dark:text-[#aaa]">
          选择一个文件查看内容。
        </div>
      </div>
    </section>
    </div>
    </template>
  </div>
</template>
