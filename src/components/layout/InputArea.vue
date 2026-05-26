<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type {
  AgentMode,
  PendingUploadFile,
  UploadedImageFile,
  UploadedDocumentFile,
  ContextUsage,
} from '../../lib/chat-types';
import {
  buildDocumentAcceptAttribute,
  extensionOf,
  parseDocumentUploadFile,
} from '../../lib/document-upload';
import { emitToast } from '../../lib/toast';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import ContextUsageIndicator from './ContextUsageIndicator.vue';

const props = defineProps<{
  isGenerating?: boolean;
  agentMode?: AgentMode;
  pendingUploads?: PendingUploadFile[];
  contextUsage?: ContextUsage;
  contextTokens?: number;
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'cancel'): void;
  (e: 'mode-change', mode: AgentMode): void;
  (e: 'upload-files', files: PendingUploadFile[]): void;
  (e: 'remove-upload', index: number): void;
}>();

const currentInput = ref("");
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);

const MAX_UPLOAD_FILE_SIZE_BYTES = 100 * 1024 * 1024;
const MAX_IMAGE_FILE_SIZE_BYTES = 5 * 1024 * 1024;
const MAX_UPLOAD_FILE_CHARS = Infinity;
const SUPPORTED_IMAGE_MIME_TYPES = new Set([
  'image/png',
  'image/jpeg',
  'image/webp',
  'image/gif',
]);
const IMAGE_EXTENSION_TO_MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  webp: 'image/webp',
  gif: 'image/gif',
};
const IMAGE_MIME_TO_EXTENSION: Record<string, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/webp': 'webp',
  'image/gif': 'gif',
};
const FILE_INPUT_ACCEPT = buildDocumentAcceptAttribute(true);

const settings = ref<any>(null);

const normalizeProviderKey = (provider: string) => (provider || '').trim().toLowerCase() || 'anthropic';

const ensureActiveProfile = () => {
  if (!settings.value) return null;
  const provider = normalizeProviderKey(settings.value.provider || 'anthropic');
  settings.value.provider = provider;
  if (!settings.value.providerProfiles || typeof settings.value.providerProfiles !== 'object') {
    settings.value.providerProfiles = {};
  }
  if (!settings.value.providerProfiles[provider]) {
    settings.value.providerProfiles[provider] = {
      displayName: '',
      protocol: provider === 'anthropic' || provider === 'dashscope-anthropic' ? 'anthropic' : 'openai',
      apiKey: '',
      baseUrl: '',
      model: '',
    };
  }
  return settings.value.providerProfiles[provider];
};

const availableModels = computed(() => {
  if (!settings.value || !settings.value.provider || !settings.value.customModels) return [];
  return settings.value.customModels[settings.value.provider] || [];
});

const currentModel = computed({
  get: () => {
    const profile = ensureActiveProfile();
    return profile?.model || '';
  },
  set: (value: string) => {
    const profile = ensureActiveProfile();
    if (!profile) return;
    profile.model = value;
  },
});

const localAgentMode = computed<AgentMode>({
  get: () => props.agentMode ?? 'agent',
  set: (value: AgentMode) => {
    emit('mode-change', value);
  },
});

const pendingUploads = computed(() => props.pendingUploads ?? []);
const hasPendingUploads = computed(() => pendingUploads.value.length > 0);
const canSend = computed(() => !!currentInput.value.trim() || hasPendingUploads.value);

// key: `${sourceName}-${index}` → 估算 token 数（异步填入）
const uploadTokenCache = ref<Map<string, number>>(new Map());

const getUploadTokenCacheKey = (file: PendingUploadFile, index: number) =>
  `${file.sourceName}-${index}`;

const formatTokenCount = (file: PendingUploadFile, index: number): string | null => {
  if (file.kind !== 'document') return null;
  const key = getUploadTokenCacheKey(file, index);
  const n = uploadTokenCache.value.get(key);
  if (n === undefined) return '…';
  if (n >= 1_000_000) return `~${(n / 1_000_000).toFixed(1)}m tokens`;
  if (n >= 1_000) return `~${(n / 1_000).toFixed(1)}k tokens`;
  return `~${n} tokens`;
};

const computeUploadTokens = async (file: PendingUploadFile, index: number) => {
  if (file.kind !== 'document') return;
  const key = getUploadTokenCacheKey(file, index);
  if (uploadTokenCache.value.has(key)) return;
  try {
    const n = await invoke<number>('estimate_text_tokens', {
      text: (file as UploadedDocumentFile).content,
      protocol: 'anthropic',
    });
    uploadTokenCache.value = new Map(uploadTokenCache.value).set(key, n);
  } catch {
    // 降级：chars / 4
    const n = Math.ceil((file as UploadedDocumentFile).content.trim().length / 4);
    uploadTokenCache.value = new Map(uploadTokenCache.value).set(key, n);
  }
};

watch(
  pendingUploads,
  (files) => {
    // 清理已移除文件的缓存
    const validKeys = new Set(files.map((f, i) => getUploadTokenCacheKey(f, i)));
    for (const k of uploadTokenCache.value.keys()) {
      if (!validKeys.has(k)) uploadTokenCache.value.delete(k);
    }
    // 计算新增文件
    files.forEach((f, i) => computeUploadTokens(f, i));
  },
  { immediate: true, deep: false },
);

const loadSettings = async () => {
  try {
    settings.value = await invoke('get_settings');
  } catch (error) {
    console.error('Failed to load settings in InputArea:', error);
  }
};

const onModelValueChange = async (value: unknown) => {
  if (typeof value !== 'string' || !settings.value) return;
  currentModel.value = value;
  try {
    await invoke('save_settings', { settings: settings.value });
  } catch (error) {
    console.error('Failed to save model change:', error);
  }
};

const inferImageMimeType = (file: File): string | null => {
  const normalizedMime = (file.type || '').trim().toLowerCase();
  if (normalizedMime && SUPPORTED_IMAGE_MIME_TYPES.has(normalizedMime)) {
    return normalizedMime;
  }
  const ext = extensionOf(file.name);
  return IMAGE_EXTENSION_TO_MIME[ext] || null;
};

const readAsDataUrl = (file: File): Promise<string> =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        resolve(reader.result);
        return;
      }
      reject(new Error('无法读取文件数据'));
    };
    reader.onerror = () => {
      reject(reader.error ?? new Error('读取文件失败'));
    };
    reader.readAsDataURL(file);
  });

const fallbackPastedImageName = (mimeType: string, index: number) => {
  const ext = IMAGE_MIME_TO_EXTENSION[mimeType] || 'png';
  return `pasted-image-${Date.now()}-${index + 1}.${ext}`;
};

const buildPendingUploadFiles = async (files: File[]): Promise<{
  accepted: PendingUploadFile[];
  rejected: string[];
}> => {
  const accepted: PendingUploadFile[] = [];
  const rejected: string[] = [];

  for (let i = 0; i < files.length; i += 1) {
    const file = files[i];
    const imageMimeType = inferImageMimeType(file);
    if (imageMimeType) {
      if (file.size > MAX_IMAGE_FILE_SIZE_BYTES) {
        rejected.push(`${file.name || `图片${i + 1}`}: 超过 5MB 图片限制`);
        continue;
      }

      let dataUrl: string;
      try {
        dataUrl = await readAsDataUrl(file);
      } catch {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片读取失败`);
        continue;
      }

      const commaIndex = dataUrl.indexOf(',');
      if (commaIndex < 0) {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片数据格式无效`);
        continue;
      }

      const base64Data = dataUrl.slice(commaIndex + 1).trim();
      if (!base64Data) {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片数据为空`);
        continue;
      }

      const imageItem: UploadedImageFile = {
        kind: 'image',
        sourceName: file.name || fallbackPastedImageName(imageMimeType, i),
        mimeType: imageMimeType,
        mediaType: imageMimeType,
        data: base64Data,
        size: file.size,
      };
      accepted.push(imageItem);
      continue;
    }

    if (file.size > MAX_UPLOAD_FILE_SIZE_BYTES) {
      rejected.push(`${file.name || `文件${i + 1}`}: 超过 100MB 限制`);
      continue;
    }

    let parsed;
    try {
      parsed = await parseDocumentUploadFile(file);
    } catch (error) {
      const message = error instanceof Error ? error.message : '文件解析失败';
      rejected.push(`${file.name || `文件${i + 1}`}: ${message}`);
      continue;
    }

    if (parsed.content.length > MAX_UPLOAD_FILE_CHARS) {
      rejected.push(`${file.name || `文件${i + 1}`}: 内容超过 ${MAX_UPLOAD_FILE_CHARS.toLocaleString()} 字符`);
      continue;
    }

    const textItem: UploadedDocumentFile = {
      kind: 'document',
      sourceName: file.name,
      mimeType: file.type || undefined,
      content: parsed.content,
      size: file.size,
      knowledgeStored: false,
    };
    accepted.push(textItem);
  }

  return { accepted, rejected };
};

const notifyRejected = (rejected: string[]) => {
  if (rejected.length <= 0) {
    return;
  }

  emitToast({
    variant: 'error',
    source: 'upload',
    message: `以下文件未导入：${rejected.slice(0, 2).join('；')}`,
  });
};

const triggerFilePicker = () => {
  if (props.isGenerating) return;
  fileInputRef.value?.click();
};

const onFileChange = async (event: Event) => {
  const input = event.target as HTMLInputElement;
  const files = input.files ? Array.from(input.files) : [];
  if (files.length === 0) {
    return;
  }

  const { accepted, rejected } = await buildPendingUploadFiles(files);

  if (accepted.length > 0) {
    emit('upload-files', accepted);
  }

  notifyRejected(rejected);

  input.value = '';
};

const onTextareaPaste = async (event: ClipboardEvent) => {
  if (props.isGenerating) return;

  const clipboardData = event.clipboardData;
  if (!clipboardData) {
    return;
  }

  const itemFiles = Array.from(clipboardData.items ?? [])
    .filter((item) => item.kind === 'file')
    .map((item) => item.getAsFile())
    .filter((file): file is File => !!file);
  const files = itemFiles.length > 0 ? itemFiles : Array.from(clipboardData.files ?? []);
  if (files.length === 0) {
    return;
  }

  const imageFiles = files.filter((file) => !!inferImageMimeType(file));
  if (imageFiles.length === 0) {
    return;
  }

  event.preventDefault();
  const { accepted, rejected } = await buildPendingUploadFiles(imageFiles);
  if (accepted.length > 0) {
    emit('upload-files', accepted);
  }
  notifyRejected(rejected);
};

const focusTextarea = () => {
  textareaRef.value?.focus();
};

const autoResize = () => {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = 'auto';
  const newHeight = Math.min(el.scrollHeight, 200);
  el.style.height = `${newHeight}px`;
};


// 发送消息，支持 Shift + Enter 换行，当 isGenerating 为 true 时禁用发送功能
const sendMessage = (e?: KeyboardEvent) => {
  if (e && e.shiftKey) return;
  e?.preventDefault();
  if ((!currentInput.value.trim() && !hasPendingUploads.value) || props.isGenerating) return;

  const message = currentInput.value.trim();
  emit('send', message);
  currentInput.value = "";
  nextTick(() => {
    autoResize();
    focusTextarea();
  });
};

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

watch(
  () => props.isGenerating,
  () => {
    nextTick(() => {
      autoResize();
      focusTextarea();
    });
  }
);


const handleSettingsUpdate = () => loadSettings();
onMounted(() => {
  loadSettings();
  window.addEventListener('settings-updated', handleSettingsUpdate);
  nextTick(() => {
    autoResize();
    focusTextarea();
  });
});

onUnmounted(() => {
  window.removeEventListener('settings-updated', handleSettingsUpdate);
});

defineExpose({
  focusTextarea,
});
</script>

<template>
  <div class="w-full">
    <input
      ref="fileInputRef"
      type="file"
      multiple
      class="hidden"
      :accept="FILE_INPUT_ACCEPT"
      @change="onFileChange"
    />
    <div
      class="relative bg-white dark:bg-[#2a2a2a] border border-[#e5e5e5] dark:border-[#3a3a3a] rounded-2xl shadow-sm focus-within:ring-2 focus-within:ring-[#e5e5e5] dark:focus-within:ring-[#444] transition-all flex flex-col w-full">
      <div v-if="hasPendingUploads" class="px-3 pt-3 pb-1">
        <div class="flex flex-wrap gap-2">
          <div
            v-for="(file, index) in pendingUploads"
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
            <span v-if="file.kind === 'document'" class="text-[11px] opacity-50">{{ formatTokenCount(file, index) }}</span>
            <span
              v-if="file.kind === 'document'"
              class="rounded-md bg-black/5 px-1.5 py-0.5 text-[10px] leading-none text-[#64748b] dark:bg-white/10 dark:text-[#cbd5e1]"
              title="文档会持续作为会话上下文；超出模型上下文窗口时自动转入会话知识库。"
            >
              会话上下文
            </span>
            <button
              type="button"
              class="w-4 h-4 inline-flex items-center justify-center rounded hover:bg-black/5 dark:hover:bg-white/10"
              @click="emit('remove-upload', index)"
            >
              <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>
        </div>
      </div>
      <textarea ref="textareaRef" v-model="currentInput" @keydown.enter="sendMessage" @input="autoResize" @paste="onTextareaPaste"
        placeholder="Message Nova..." rows="1"
        class="w-full bg-transparent border-none text-[0.95rem] text-[#1a1a1a] dark:text-[#ececec] resize-none outline-none block max-h-[40vh] px-4 pt-3 pb-2 placeholder:text-[#a3a3a3]"></textarea>

      <div class="flex min-w-0 items-center gap-2 px-3 pb-3 pt-2">
        <div class="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <button
            type="button"
            class="w-8 h-8 shrink-0 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-secondary/80 transition-colors"
            @click="triggerFilePicker">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
              stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 5v14M5 12h14" />
            </svg>
          </button>

          <div class="w-[92px] shrink-0">
            <Select v-model="localAgentMode">
              <SelectTrigger size="sm" class="w-full text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent class="text-xs">
                <SelectItem value="agent">Agent</SelectItem>
                <SelectItem value="plan">Plan</SelectItem>
                <SelectItem value="auto">Auto</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div v-if="availableModels.length > 0 && settings" class="flex min-w-0 shrink-0 items-center gap-1.5">
            <Select :model-value="currentModel" @update:model-value="onModelValueChange">
              <SelectTrigger size="sm" class="w-[150px] max-w-[28vw] text-xs">
                <SelectValue placeholder="选择模型" />
              </SelectTrigger>
              <SelectContent class="text-xs">
                <SelectItem v-for="model in availableModels" :key="model" :value="model">
                  {{ model }}
                </SelectItem>
              </SelectContent>
            </Select>
            <ContextUsageIndicator :usage="contextUsage" :usedTokens="contextTokens" :model="currentModel" />
          </div>
        </div>
        <button class="w-8 h-8 shrink-0 rounded-full flex items-center justify-center transition-colors shadow-sm"
          :class="isGenerating
            ? 'bg-[#fee2e2] text-[#b91c1c] hover:bg-[#fecaca]'
            : (canSend ? 'bg-[#111827] text-white hover:bg-[#1f2937]' : 'bg-[#f1f5f9] dark:bg-[#333] text-muted-foreground')"
          :disabled="!isGenerating && !canSend" @click="isGenerating ? emit('cancel') : sendMessage()">
          <svg v-if="isGenerating" width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
            <rect x="6" y="6" width="12" height="12" rx="2" ry="2" />
          </svg>
          <svg v-else-if="!canSend" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
            <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
            <line x1="12" y1="19" x2="12" y2="22" />
          </svg>
          <svg v-else width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="19" x2="12" y2="5" />
            <polyline points="5 12 12 5 19 12" />
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>
