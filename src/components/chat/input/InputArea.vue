<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch, computed } from 'vue';
import { parseSlashCommand } from '@/lib/slash-commands';
import { getRuntimeSettings } from '@/features/chat/services/chat-api';
import { useConversationStore } from '@/stores/conversation';
import { useAgentSessionStore } from '@/stores/agentSession';
import { useComposerStore } from '@/stores/composer';
import PolicyApprovalMenu from './PolicyApprovalMenu.vue';
import ModelSelector from './ModelSelector.vue';
import ContextUsageIndicator from './ContextUsageIndicator.vue';
import AttachmentChipList from './AttachmentChipList.vue';
import SlashCommandMenu from './SlashCommandMenu.vue';
import MemoryPopover from './MemoryPopover.vue';
import { initSubagentEvents } from '@/features/chat/services/subagents';
import { useFileInput } from './composer/useFileInput';
import { useSlashCommands, type SkillSummary } from './composer/useSlashCommands';
import InputPlusMenu from './composer/InputPlusMenu.vue';
import InputAreaFooter from './composer/InputAreaFooter.vue';

defineProps<{
  /** 当前对话挂载的智能体（会话级）。null = 默认 Nova（不展示）。 */
  activeAgent?: { id: string; name: string; description?: string } | null;
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'cancel'): void;
  (e: 'compact'): void;
  (e: 'remove-agent'): void;
}>();

const conversationStore = useConversationStore();
const sessionStore = useAgentSessionStore();
const composerStore = useComposerStore();

const isGenerating = computed(() => sessionStore.activeSession.isGenerating);
const compacting = computed(() => sessionStore.activeSession.isCompacting);
const conversationId = computed(() => conversationStore.activeConversationId);
const contextUsage = computed(() => sessionStore.activeSession.contextUsage);
const contextTokens = computed(() => sessionStore.activeSession.contextTokens);
const conversationUsage = computed(() => sessionStore.activeSession.conversationUsage);
const pendingUploads = computed(() => composerStore.pendingUploads);

const currentInput = computed({
  get: () => composerStore.currentInput,
  set: (val: string) => composerStore.setInput(val),
});
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const plusButtonRef = ref<HTMLElement | null>(null);
const isComposing = computed({
  get: () => composerStore.isComposing,
  set: (val: boolean) => {
    composerStore.isComposing = val;
  },
});

// + 按钮菜单状态：null=关闭，'main'=主视图，'skill'=技能视图
const plusMenuView = ref<null | 'main' | 'skill'>(null);
const skills = ref<SkillSummary[]>([]);
const skillsLoading = ref(false);

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
      protocol: provider === 'anthropic' ? 'anthropic' : 'openai',
      apiKey: '',
      baseUrl: '',
      model: '',
    };
  }
  return settings.value.providerProfiles[provider];
};

const currentModel = computed(() => {
  const profile = ensureActiveProfile();
  return profile?.model || '';
});

const hasPendingUploads = computed(() => pendingUploads.value.length > 0);
const canSend = computed(() => !!currentInput.value.trim() || hasPendingUploads.value);

const loadSettings = async () => {
  try {
    settings.value = await getRuntimeSettings();
  } catch (error) {
    console.error('Failed to load settings in InputArea:', error);
  }
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

// 组合式文件输入
const {
  fileInputRef,
  FILE_INPUT_ACCEPT,
  triggerFilePicker,
  onFileChange,
  onTextareaPaste,
  handleRemoveUpload,
} = useFileInput({
  isGenerating,
});

// 组合式斜杠命令
const {
  slashPhase,
  slashSelectedIndex,
  slashSkills,
  slashSkillsLoading,
  slashActiveCommand,
  slashQuery,
  slashOptions,
  memoryEntries,
  memoryLoading,
  memoryViewOpen,
  loadSkills,
  refreshSlashState,
  hideSlashMenu,
  selectSlashOption,
  handleSlashKeydown,
  executeSlashCommand,
} = useSlashCommands({
  currentInput,
  textareaRef,
  conversationId,
  compacting,
  onSend: (msg) => emit('send', msg),
  onCompact: () => emit('compact'),
  autoResize,
  focusTextarea,
});

// + 按钮操作
const openPlusMenu = async () => {
  if (isGenerating.value) return;
  plusMenuView.value = 'main';
  if (skills.value.length === 0 && !skillsLoading.value) {
    skillsLoading.value = true;
    skills.value = await loadSkills();
    skillsLoading.value = false;
  }
};

const closePlusMenu = () => {
  plusMenuView.value = null;
};

const enterSkillView = async () => {
  plusMenuView.value = 'skill';
  if (slashSkills.value.length === 0 && !slashSkillsLoading.value) {
    slashSkillsLoading.value = true;
    slashSkills.value = await loadSkills();
    skills.value = slashSkills.value;
    slashSkillsLoading.value = false;
  }
};

const pickUploadFromPlusMenu = () => {
  closePlusMenu();
  triggerFilePicker();
};

const pickSkillFromPlusMenu = (skill: SkillSummary) => {
  currentInput.value = `/skill ${skill.name} `;
  closePlusMenu();
  slashActiveCommand.value = 'skill';
  slashPhase.value = 'param';
  slashQuery.value = skill.name;
  nextTick(() => {
    autoResize();
    focusTextarea();
    const el = textareaRef.value;
    if (el) {
      const len = el.value.length;
      el.setSelectionRange(len, len);
    }
  });
};

/** 把选中文本以 markdown 引用块形式插入输入框末尾（「引用到对话」入口）。 */
const insertQuotedText = (text: string) => {
  const quote = text
    .trim()
    .split('\n')
    .map((line) => `> ${line}`)
    .join('\n');
  if (!quote) return;
  const existing = currentInput.value.trimEnd();
  currentInput.value = existing ? `${existing}\n\n${quote}\n\n` : `${quote}\n\n`;
  void nextTick(() => {
    autoResize();
    const el = textareaRef.value;
    if (el) {
      el.focus();
      el.selectionStart = el.value.length;
      el.selectionEnd = el.value.length;
    }
  });
};

const onTextareaInput = () => {
  autoResize();
  refreshSlashState();
};

const onTextareaKeydown = (e: KeyboardEvent) => {
  if (slashPhase.value !== null) {
    if (handleSlashKeydown(e)) return;
  }
  if (e.key === 'Enter' && !e.shiftKey && !isComposing.value) {
    sendMessage(e);
  }
};

const sendMessage = (e?: KeyboardEvent) => {
  if (e && e.shiftKey) return;
  e?.preventDefault();
  if ((!currentInput.value.trim() && !hasPendingUploads.value) || isGenerating.value) return;

  const trimmed = currentInput.value.trim();
  const parsed = parseSlashCommand(trimmed);
  if (parsed) {
    const { entry, rest } = parsed;
    if (entry.args === 'options' && !rest) {
      return;
    }
    currentInput.value = '';
    hideSlashMenu();
    nextTick(() => {
      autoResize();
      focusTextarea();
    });
    void executeSlashCommand(parsed);
    return;
  }

  const message = trimmed;
  emit('send', message);
  composerStore.clearUploads();
  currentInput.value = "";
  hideSlashMenu();
  nextTick(() => {
    autoResize();
    focusTextarea();
  });
};

watch(
  isGenerating,
  () => {
    nextTick(() => {
      autoResize();
      focusTextarea();
    });
  }
);

const handleSettingsUpdate = () => loadSettings();

const handleDocumentClick = (e: MouseEvent) => {
  const target = e.target as Node | null;
  if (plusMenuView.value !== null) {
    if (plusButtonRef.value && target && plusButtonRef.value.contains(target)) return;
    const menus = document.querySelectorAll('[data-plus-menu]');
    for (const menu of menus) {
      if (menu.contains(target)) return;
    }
    closePlusMenu();
  }
  if (memoryViewOpen.value) {
    const memMenu = document.querySelector('[data-memory-menu]');
    if (memMenu && target && memMenu.contains(target)) return;
    memoryViewOpen.value = false;
  }
};

onMounted(() => {
  void initSubagentEvents();
  loadSettings();
  window.addEventListener('settings-updated', handleSettingsUpdate);
  document.addEventListener('click', handleDocumentClick, true);
  nextTick(() => {
    autoResize();
    focusTextarea();
  });
});

onUnmounted(() => {
  window.removeEventListener('settings-updated', handleSettingsUpdate);
  document.removeEventListener('click', handleDocumentClick, true);
});

defineExpose({
  focusTextarea,
  insertQuotedText,
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
      class="relative bg-white dark:bg-[#2a2a2a] border border-[#e5e5e5] dark:border-[#3a3a3a] rounded-2xl shadow-sm focus-within:ring-2 focus-within:ring-[#e5e5e5] dark:focus-within:ring-[#444] transition-all flex flex-col w-full"
    >
      <AttachmentChipList :files="pendingUploads" @remove="handleRemoveUpload" />
      <div class="relative w-full">
        <textarea
          ref="textareaRef"
          v-model="currentInput"
          @keydown="onTextareaKeydown"
          @input="onTextareaInput"
          @paste="onTextareaPaste"
          @compositionstart="isComposing = true"
          @compositionend="isComposing = false"
          placeholder="Message Nova..."
          rows="1"
          class="relative w-full bg-transparent border-none text-[0.95rem] text-[#1a1a1a] dark:text-[#ececec] caret-[#1a1a1a] dark:caret-[#ececec] resize-none outline-none block max-h-[40vh] px-4 pt-3 pb-2 placeholder:text-[#a3a3a3]"
        ></textarea>

        <!-- 斜杠命令下拉菜单：向上弹出，与输入框同宽 -->
        <SlashCommandMenu
          :visible="slashPhase !== null"
          :options="slashOptions"
          :selectedIndex="slashSelectedIndex"
          :loading="slashPhase === 'param' && slashSkillsLoading"
          @select="selectSlashOption"
          @update:selectedIndex="slashSelectedIndex = $event"
        />

        <!-- /memory 浮层：展示全局记忆条目（与输入框同宽） -->
        <MemoryPopover
          :open="memoryViewOpen"
          :loading="memoryLoading"
          :entries="memoryEntries"
          @close="memoryViewOpen = false"
        />

        <!-- + 按钮菜单（主视图 / 技能视图） -->
        <InputPlusMenu
          :view="plusMenuView"
          :skills="skills"
          :loading="skillsLoading"
          @upload="pickUploadFromPlusMenu"
          @enter-skill="enterSkillView"
          @back-main="plusMenuView = 'main'"
          @select-skill="pickSkillFromPlusMenu"
        />
      </div>

      <div class="flex min-w-0 items-center gap-2 px-3 pb-3 pt-2">
        <div class="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <button
            ref="plusButtonRef"
            type="button"
            class="w-8 h-8 shrink-0 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-secondary/80 transition-colors"
            :class="{ 'bg-secondary/80': plusMenuView !== null }"
            @click="plusMenuView !== null ? closePlusMenu() : openPlusMenu()"
          >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
              stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 5v14M5 12h14" />
            </svg>
          </button>

          <!-- 审批策略快捷菜单 -->
          <PolicyApprovalMenu
            :settings="settings"
            @update:settings="settings = $event"
          />

          <!-- 模型选择器与用量指示器 -->
          <div v-if="settings" class="flex min-w-0 shrink-0 items-center gap-1.5">
            <ModelSelector
              :settings="settings"
              @update:settings="settings = $event"
            />
            <ContextUsageIndicator
              :usage="contextUsage"
              :usedTokens="contextTokens"
              :model="currentModel"
              :compacting="compacting"
              @compact="emit('compact')"
            />
          </div>
        </div>
        <button
          class="w-8 h-8 shrink-0 rounded-full flex items-center justify-center transition-colors shadow-sm"
          :class="isGenerating
            ? 'bg-[#fee2e2] text-[#b91c1c] hover:bg-[#fecaca]'
            : (canSend ? 'bg-[#111827] text-white hover:bg-[#1f2937]' : 'bg-[#f1f5f9] dark:bg-[#333] text-muted-foreground')"
          :disabled="!isGenerating && !canSend"
          @click="isGenerating ? emit('cancel') : sendMessage()"
        >
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

    <!-- 输入框卡片外部下方状态行：由 InputAreaFooter 负责 -->
    <InputAreaFooter
      :activeAgent="activeAgent"
      :conversationId="conversationId"
      :conversationUsage="conversationUsage"
      @remove-agent="emit('remove-agent')"
    />
  </div>
</template>
