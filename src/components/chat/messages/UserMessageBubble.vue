<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import type { ChatMessage } from '../../../lib/chat-types';

const props = defineProps<{
  message: ChatMessage;
  index: number;
  copied: boolean;
  timeText: string;
}>();

const emit = defineEmits<{
  (e: 'retry', index: number): void;
  (e: 'save-edit', payload: { index: number; content: string }): void;
  (e: 'copy', index: number): void;
}>();

const isExpanded = ref(false);
const isEditing = ref(false);
const editDraft = ref('');
const editTextareaRef = ref<HTMLTextAreaElement | null>(null);

const normalizedContent = computed(() => props.message.content.trim());
const lineCount = computed(() => normalizedContent.value.split(/\r?\n/).length);
const shouldCollapse = computed(() => {
  if (!normalizedContent.value) return false;
  return normalizedContent.value.length > 260 || lineCount.value > 8;
});

const toggleExpanded = () => {
  if (!shouldCollapse.value) return;
  isExpanded.value = !isExpanded.value;
};

const isEditUnchanged = computed(
  () => editDraft.value.trim() === props.message.content.trim(),
);

const beginEdit = async () => {
  editDraft.value = props.message.content;
  isEditing.value = true;
  await nextTick();
  editTextareaRef.value?.focus();
  editTextareaRef.value?.select();
};

const cancelEdit = () => {
  isEditing.value = false;
  editDraft.value = props.message.content;
};

const saveEdit = () => {
  const content = editDraft.value.trim();
  if (!content || isEditUnchanged.value) {
    return;
  }
  isEditing.value = false;
  emit('save-edit', { index: props.index, content });
};

watch(
  () => props.message.content,
  (next) => {
    if (!isEditing.value) {
      editDraft.value = next;
    }
  },
  { immediate: true },
);

const formatFileSize = (bytes?: number) => {
  if (!bytes || !Number.isFinite(bytes) || bytes <= 0) {
    return '';
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
</script>

<template>
  <div
    class="ml-auto flex max-w-full items-start"
    :class="isEditing ? 'w-full max-w-[calc(100%-2.5rem)]' : 'max-w-[86%] sm:max-w-[78%] lg:max-w-[66%]'"
  >
    <div class="flex max-w-full flex-col items-end min-w-0" :class="{ 'flex-1': isEditing }">
      <span v-if="typeof message.tokenUsage === 'number'" class="token-badge">
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
            <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
            <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
          </svg>
          本次 {{ message.tokenUsage ?? 0 }}
      </span>
      <div
        :class="
          isEditing
            ? 'edit-shell'
            : 'max-w-full overflow-hidden bg-[#f3f3f3] dark:bg-[#2d2d2d] px-4 py-2.5 rounded-xl border border-[#e5e7eb] dark:border-[#3c3c3c]'
        "
      >
        <div v-if="message.attachments?.length" class="mb-2 flex flex-wrap gap-1.5">
          <div
            v-for="(file, i) in message.attachments"
            :key="`${file.sourceName}-${i}`"
            class="inline-flex items-center gap-1.5 rounded-md border border-[#e5e7eb] dark:border-[#4a4a4a] bg-white dark:bg-[#353535] px-2 py-1 text-[11px] text-[#475569] dark:text-[#d7d0c5]"
          >
            <svg
              v-if="file.kind === 'image'"
              width="11"
              height="11"
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
              width="11"
              height="11"
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
            <span class="max-w-[180px] truncate" :title="file.sourceName">{{ file.sourceName }}</span>
            <span v-if="formatFileSize(file.size)" class="opacity-70">{{ formatFileSize(file.size) }}</span>
          </div>
        </div>
        <div v-if="isEditing" class="edit-card">
          <textarea
            ref="editTextareaRef"
            v-model="editDraft"
            rows="3"
            class="edit-card__textarea"
          ></textarea>
          <div class="edit-card__footer">
            <div class="edit-card__notice">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <circle cx="12" cy="12" r="9"></circle>
                <path d="M12 8h.01"></path>
                <path d="M11 12h1v4h1"></path>
              </svg>
              <span>编辑后会从这条消息重新生成后续回复。</span>
            </div>
            <div class="edit-card__actions">
              <button type="button" class="edit-card__btn edit-card__btn--ghost" @click="cancelEdit">
                取消
              </button>
              <button
                type="button"
                class="edit-card__btn edit-card__btn--primary"
                :disabled="!editDraft.trim() || isEditUnchanged"
                @click="saveEdit"
              >
                发送
              </button>
            </div>
          </div>
        </div>
        <template v-else>
          <div
            v-if="normalizedContent"
            class="user-message-text text-[0.92rem] leading-relaxed whitespace-pre-wrap text-[#111827] dark:text-[#ececec]"
            :class="{ 'is-collapsed': shouldCollapse && !isExpanded }"
          >
            {{ message.content }}
          </div>
          <button
            v-if="shouldCollapse"
            type="button"
            class="user-message-toggle"
            @click="toggleExpanded"
          >
            {{ isExpanded ? '收起' : '展开全文' }}
          </button>
        </template>
      </div>
      <div class="msg-toolbar">
        <span class="msg-time">{{ timeText }}</span>
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" aria-label="Retry user message" @click="emit('retry', index)">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" aria-label="Edit message" @click="beginEdit">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
        </Button>
        <Button variant="ghost" size="icon-sm" class="msg-icon-btn" :class="{ 'is-copied': copied }" aria-label="Copy message" @click="emit('copy', index)">
          <svg v-if="!copied" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
          <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        </Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.msg-toolbar {
  display: flex;
  align-items: center;
  gap: 1px;
  margin-top: 4px;
  padding: 0 1px;
}

.msg-time {
  font-size: 11px;
  color: #94a3b8;
  margin-right: 4px;
  font-variant-numeric: tabular-nums;
}

.msg-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  border-radius: 5px;
  color: #94a3b8;
  cursor: pointer;
  transition: color 0.15s, background 0.15s;
}

.msg-icon-btn:hover {
  color: #334155;
  background: #f1f5f9;
}

.msg-icon-btn.is-copied {
  color: #4a7c59;
}

.token-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 9px;
  color: #047857;
  border: 1px solid #a7f3d0;
  background: #ecfdf5;
  padding: 3px 6px;
  border-radius: 6px;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Mono', monospace;
  letter-spacing: 0.03em;
  font-variant-numeric: tabular-nums;
}

.dark .token-badge {
  color: #86efac;
  border-color: rgba(34, 197, 94, 0.38);
  background: rgba(20, 83, 45, 0.32);
}

.user-message-text {
  position: relative;
  max-width: 100%;
  overflow-wrap: anywhere;
  word-break: break-word;
}

.user-message-text.is-collapsed {
  max-height: 9.4em;
  overflow: hidden;
  mask-image: linear-gradient(180deg, #000 0%, #000 72%, transparent 100%);
  -webkit-mask-image: linear-gradient(180deg, #000 0%, #000 72%, transparent 100%);
}

.user-message-toggle {
  margin-top: 8px;
  padding: 0;
  border: none;
  background: transparent;
  color: #64748b;
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
  transition: color 0.15s ease;
}

.user-message-toggle:hover {
  color: #334155;
}

.dark .user-message-toggle {
  color: #cbd5e1;
}

.dark .user-message-toggle:hover {
  color: #f8fafc;
}

.edit-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.edit-card__textarea {
  width: 100%;
  min-height: 88px;
  resize: vertical;
  border-radius: 18px;
  border: 2px solid #2f74d3;
  background: rgba(255, 255, 255, 0.96);
  padding: 16px 18px;
  font-size: 14px;
  line-height: 1.55;
  color: #111827;
  outline: none;
  box-shadow: 0 0 0 4px rgba(47, 116, 211, 0.1);
}

.edit-card__footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
}

.edit-card__notice {
  display: inline-flex;
  align-items: flex-start;
  gap: 8px;
  color: #64748b;
  font-size: 12px;
  line-height: 1.45;
}

.edit-card__actions {
  display: inline-flex;
  align-items: center;
  gap: 12px;
}

.edit-card__btn {
  min-width: 92px;
  height: 42px;
  border-radius: 15px;
  border: 1px solid transparent;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.16s ease, color 0.16s ease, border-color 0.16s ease, opacity 0.16s ease;
}

.edit-card__btn--ghost {
  background: rgba(255, 255, 255, 0.6);
  border-color: #cbd5e1;
  color: #111827;
}

.edit-card__btn--ghost:hover {
  background: rgba(255, 255, 255, 0.9);
}

.edit-card__btn--primary {
  background: #111827;
  color: white;
}

.edit-card__btn--primary:not(:disabled):hover {
  background: #1f2937;
}

.edit-card__btn--primary:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.dark .edit-card__textarea {
  background: rgba(32, 31, 28, 0.96);
  border-color: #5d8fda;
  box-shadow: 0 0 0 4px rgba(93, 143, 218, 0.12);
  color: #ececec;
}

.dark .edit-card__notice {
  color: #94a3b8;
}

.dark .edit-card__btn--ghost {
  background: rgba(255, 255, 255, 0.02);
  border-color: #475569;
  color: #e2e8f0;
}

.dark .edit-card__btn--ghost:hover {
  background: rgba(255, 255, 255, 0.06);
}

.dark .edit-card__btn--primary {
  background: #e5e7eb;
  color: #111827;
}

.dark .edit-card__btn--primary:not(:disabled):hover {
  background: #f3f4f6;
}

.edit-shell {
  width: 100%;
  padding: 14px 16px 10px;
  border-radius: 18px;
  border: 1px solid #e5e7eb;
  background: #f8fafc;
  box-shadow: 0 8px 24px rgba(15, 23, 42, 0.06);
}

.dark .edit-shell {
  border-color: #3f4652;
  background: #262b33;
  box-shadow: 0 10px 24px rgba(0, 0, 0, 0.22);
}

@media (max-width: 900px) {
  .edit-shell {
    padding: 12px 12px 10px;
  }

  .edit-card__footer {
    flex-direction: column;
    align-items: stretch;
  }

  .edit-card__actions {
    justify-content: flex-end;
  }
}
</style>
