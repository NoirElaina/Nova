<script setup lang="ts">
import InputArea from '../layout/InputArea.vue';
import EnvironmentBar from './EnvironmentBar.vue';
import type {
  ContextCompactSummary,
  ContextUsage,
  PendingUploadFile,
} from '../../lib/chat-types';

defineProps<{
  isGenerating?: boolean;
  pendingUploads?: PendingUploadFile[];
  contextUsage?: ContextUsage;
  contextCompacts?: ContextCompactSummary[];
  contextTokens?: number;
  workspacePath?: string;
  conversationId?: string | null;
  /** 暂存的智能体（会话未创建时展示，首次发送时挂载到新对话）。 */
  activeAgent?: { id: string; name: string; description?: string } | null;
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'remove-agent'): void;
  (e: 'upload-files', files: PendingUploadFile[]): void;
  (e: 'remove-upload', index: number): void;
  (e: 'update:workspacePath', path: string): void;
}>();

const handleSend = (msg: string) => {
  emit('send', msg);
};
</script>

<template>
  <div class="flex-1 flex flex-col items-center justify-center pt-10 px-4 w-full h-full">

    <h1 class="text-4xl text-[#1a1a1a] dark:text-[#ececec] font-serif mb-8 flex items-center justify-center gap-4 tracking-tight">
      <svg class="text-[#2563eb]" width="38" height="38" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 2v20M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/>
      </svg>
      Back at it, Nova
    </h1>

    <div class="w-full max-w-[42rem] flex flex-col">
      <EnvironmentBar
        :workspacePath="workspacePath"
        :conversationId="conversationId"
        @update:workspacePath="emit('update:workspacePath', $event)"
      />

      <InputArea
        :isGenerating="isGenerating"
        :pendingUploads="pendingUploads"
        :contextUsage="contextUsage"
        :contextTokens="contextTokens"
        :activeAgent="activeAgent"
        @send="handleSend"
        @remove-agent="emit('remove-agent')"
        @upload-files="emit('upload-files', $event)"
        @remove-upload="emit('remove-upload', $event)"
      />
    </div>
  </div>
</template>
