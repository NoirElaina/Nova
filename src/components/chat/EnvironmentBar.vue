<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { emitToast } from '../../lib/toast';
import { getWorkspaceGitStatus } from '../../features/chat/services/chat-api';

const props = defineProps<{
  workspacePath?: string;
}>();

const emit = defineEmits<{
  (e: 'update:workspacePath', path: string): void;
}>();

const isChangingWorkspace = ref(false);
const gitBranch = ref<string | null>(null);
const gitWorktree = ref<string | null>(null);

const workspaceName = computed(() => {
  const path = props.workspacePath?.trim();
  if (!path) return 'Nova';
  const parts = path.replace(/\\/g, '/').split('/');
  return parts[parts.length - 1] || 'Nova';
});

const displayName = computed(() => {
  const name = workspaceName.value;
  if (name.length > 20) return name.slice(0, 18) + '…';
  return name;
});

async function refreshGitStatus(path: string) {
  const trimmed = path.trim();
  if (!trimmed) {
    gitBranch.value = null;
    gitWorktree.value = null;
    return;
  }
  try {
    const status = await getWorkspaceGitStatus(trimmed);
    gitBranch.value = status.initialized ? status.branch ?? null : null;
    gitWorktree.value = status.initialized ? status.worktree ?? null : null;
  } catch (err) {
    console.error('Failed to query git status:', err);
    gitBranch.value = null;
    gitWorktree.value = null;
  }
}

watch(
  () => props.workspacePath,
  (path) => {
    if (path) void refreshGitStatus(path);
    else {
      gitBranch.value = null;
      gitWorktree.value = null;
    }
  },
  { immediate: true },
);

const handleChangeWorkspace = async () => {
  if (isChangingWorkspace.value) return;
  try {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: '选择工作区',
    });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path || typeof path !== 'string') return;

    isChangingWorkspace.value = true;
    emit('update:workspacePath', path);
    emitToast({ variant: 'success', source: 'workspace', message: '工作区已切换。' });
  } catch (error) {
    console.error('Failed to change workspace root:', error);
    emitToast({ variant: 'error', source: 'workspace', message: '更换工作区失败。' });
  } finally {
    isChangingWorkspace.value = false;
  }
};
</script>

<template>
  <div class="flex items-center gap-2 px-1 pb-2 text-[13px] text-[#64748b] dark:text-[#9ca3af]">
    <button
      type="button"
      class="inline-flex items-center gap-1.5 rounded-md px-2 py-1 hover:bg-black/5 dark:hover:bg-white/8 transition-colors"
      title="本地工作区"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2" ry="2"/>
        <line x1="8" y1="21" x2="16" y2="21"/>
        <line x1="12" y1="17" x2="12" y2="21"/>
      </svg>
      <span>Local</span>
    </button>

    <button
      type="button"
      class="inline-flex items-center gap-1.5 rounded-md px-2 py-1 hover:bg-black/5 dark:hover:bg-white/8 transition-colors"
      :title="workspacePath"
      :disabled="isChangingWorkspace"
      @click="handleChangeWorkspace"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
      </svg>
      <span>{{ displayName }}</span>
    </button>

    <span v-if="gitBranch || gitWorktree" class="text-[#d1d5db] dark:text-[#4b5563]">|</span>

    <button
      v-if="gitBranch"
      type="button"
      class="inline-flex items-center gap-1.5 rounded-md px-2 py-1 hover:bg-black/5 dark:hover:bg-white/8 transition-colors"
      title="Git 分支"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <line x1="6" y1="3" x2="6" y2="15"/>
        <circle cx="18" cy="6" r="3"/>
        <circle cx="6" cy="18" r="3"/>
        <path d="M18 9a9 9 0 0 1-9 9"/>
      </svg>
      <span>{{ gitBranch }}</span>
    </button>

    <button
      v-if="gitWorktree"
      type="button"
      class="inline-flex items-center gap-1.5 rounded-md px-2 py-1 hover:bg-black/5 dark:hover:bg-white/8 transition-colors"
      title="Worktree"
    >
      <span class="inline-block w-2.5 h-2.5 rounded-sm bg-[#a3a3a3] dark:bg-[#6b7280]"/>
      <span>{{ gitWorktree }}</span>
    </button>

    <button
      type="button"
      class="ml-1 inline-flex items-center justify-center w-6 h-6 rounded-md hover:bg-black/5 dark:hover:bg-white/8 transition-colors"
      title="刷新"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="23 4 23 10 17 10"/>
        <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
      </svg>
    </button>

    <div class="flex-1"/>

    <div class="flex items-center justify-center w-7 h-7" title="Nova Pet">
      <svg width="24" height="24" viewBox="0 0 24 24" fill="#c97b5a">
        <rect x="4" y="6" width="16" height="10" rx="2"/>
        <rect x="6" y="3" width="3" height="4" rx="1"/>
        <rect x="15" y="3" width="3" height="4" rx="1"/>
        <rect x="7" y="9" width="2.5" height="2.5" rx="0.5" fill="#1a1a1a"/>
        <rect x="14.5" y="9" width="2.5" height="2.5" rx="0.5" fill="#1a1a1a"/>
        <rect x="10" y="12" width="4" height="1.5" rx="0.5" fill="#1a1a1a"/>
        <rect x="5" y="16" width="3" height="3" rx="1"/>
        <rect x="16" y="16" width="3" height="3" rx="1"/>
      </svg>
    </div>
  </div>
</template>
