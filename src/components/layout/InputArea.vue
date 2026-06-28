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
import { emitToast, emitErrorToast } from '../../lib/toast';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import ContextUsageIndicator from './ContextUsageIndicator.vue';
import { getWorkspaceDiff } from '../../features/chat/services/chat-api';
import {
  SLASH_COMMANDS,
  MEMORY_OPTIONS,
  REVIEW_OPTIONS,
  INIT_OPTIONS,
  parseSlashCommand,
  buildInitPrompt,
  buildReviewPrompt,
  formatWorkspaceDiff,
} from '../../lib/slash-commands';
import type {
  SlashCommandEntry,
  SlashParamOption,
} from '../../lib/slash-commands';

type SkillSummary = {
  name: string;
  description: string;
  path: string;
};

const props = defineProps<{
  isGenerating?: boolean;
  agentMode?: AgentMode;
  pendingUploads?: PendingUploadFile[];
  contextUsage?: ContextUsage;
  contextTokens?: number;
  compacting?: boolean;
}>();

const emit = defineEmits<{
  (e: 'send', msg: string): void;
  (e: 'cancel'): void;
  (e: 'mode-change', mode: AgentMode): void;
  (e: 'upload-files', files: PendingUploadFile[]): void;
  (e: 'remove-upload', index: number): void;
  (e: 'compact'): void;
}>();

const currentInput = ref("");
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);
// IME 合成状态：中文输入法打拼音过程中为 true，此时 textarea 文字需可见，
// 否则被 text-transparent 隐藏导致用户看不见正在输入的字母。
const isComposing = ref(false);

// + 按钮菜单状态：null=关闭，'main'=主视图，'skill'=技能视图
const plusMenuView = ref<null | 'main' | 'skill'>(null);
const skills = ref<SkillSummary[]>([]);
const skillsLoading = ref(false);

// 斜杠命令状态：null=未触发，'command'=命令列表阶段，'param'=参数匹配阶段
const slashPhase = ref<null | 'command' | 'param'>(null);
const slashQuery = ref('');
const slashSelectedIndex = ref(0);
const slashSkills = ref<SkillSummary[]>([]);
const slashSkillsLoading = ref(false);

const plusButtonRef = ref<HTMLElement | null>(null);

// 当前选中的命令名（进入 param 阶段后固定）
const slashActiveCommand = ref<string>('');

// /memory 浮层状态
const memoryEntries = ref<string[]>([]);
const memoryLoading = ref(false);
const memoryViewOpen = ref(false);

// /review 执行中标记（异步拿 diff）
const reviewLoading = ref(false);

// 各命令二级选项动态数据：skill 需加载技能列表，compact 需加载用量统计
const usageStats = ref<{ total_tokens: number; total_cost_usd: string; favorite_model?: string } | null>(null);
const usageLoading = ref(false);

const MAX_UPLOAD_FILE_SIZE_BYTES = 100 * 1024 * 1024;
// 图片最终 base64 编码后的上限（约 5MB），超出会自动缩放
const MAX_IMAGE_BASE64_BYTES = 5 * 1024 * 1024;
const IMAGE_MAX_DIMENSION = 2000;
const IMAGE_RESIZE_SCALE_STEPS = 0.75;
const IMAGE_JPEG_QUALITIES = [0.85, 0.7, 0.55, 0.4];
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
      protocol: provider === 'anthropic' ? 'anthropic' : 'openai',
      apiKey: '',
      baseUrl: '',
      model: '',
    };
  }
  return settings.value.providerProfiles[provider];
};

const availableModels = computed(() => {
  if (!settings.value?.provider) return [];
  const provider = normalizeProviderKey(settings.value.provider);
  const listed = settings.value.customModels?.[provider];
  if (Array.isArray(listed) && listed.length > 0) {
    return listed;
  }
  const profile = settings.value.providerProfiles?.[provider];
  const fallbackModel = typeof profile?.model === 'string' ? profile.model.trim() : '';
  return fallbackModel ? [fallbackModel] : [];
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

const loadSettings = async () => {
  try {
    settings.value = await invoke('get_settings');
  } catch (error) {
    console.error('Failed to load settings in InputArea:', error);
  }
};

// 加载技能列表（用于 + 菜单和斜杠命令参数匹配）
const loadSkills = async (): Promise<SkillSummary[]> => {
  try {
    const list = await invoke<SkillSummary[]>('list_skills');
    return list || [];
  } catch (error) {
    console.error('Failed to load skills:', error);
    return [];
  }
};

// + 按钮点击：打开主视图，按需预加载技能列表
const openPlusMenu = async () => {
  if (props.isGenerating) return;
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

// + 菜单选择"上传文件"
const pickUploadFromPlusMenu = () => {
  closePlusMenu();
  triggerFilePicker();
};

// + 菜单选择某个技能：填入 /skill <name> 到输入框
const pickSkillFromPlusMenu = (skill: SkillSummary) => {
  currentInput.value = `/skill ${skill.name} `;
  closePlusMenu();
  // 进入参数阶段，便于继续编辑/补充参数
  slashActiveCommand.value = 'skill';
  slashPhase.value = 'param';
  slashQuery.value = skill.name;
  nextTick(() => {
    autoResize();
    focusTextarea();
    // 光标移到末尾
    const el = textareaRef.value;
    if (el) {
      const len = el.value.length;
      el.setSelectionRange(len, len);
    }
  });
};

// ── 斜杠命令逻辑 ──────────────────────────────────────────────────

// 命令列表阶段的过滤结果
const filteredCommands = computed(() => {
  const q = slashQuery.value.trim().toLowerCase();
  if (!q) return SLASH_COMMANDS;
  return SLASH_COMMANDS.filter((cmd) => cmd.name.toLowerCase().includes(q));
});

// 获取指定命令的二级选项列表（含动态加载）
const currentParamOptions = computed<SlashParamOption[]>(() => {
  const cmd = slashActiveCommand.value;
  if (cmd === 'skill') {
    // skill 二级选项为技能列表
    return slashSkills.value.map((s) => ({
      label: s.name,
      description: s.description,
      value: s.name,
    }));
  }
  if (cmd === 'compact') {
    // compact 二级选项：展示用量统计后压缩
    const usage = usageStats.value;
    if (usage) {
      return [
        {
          label: '继续压缩',
          value: 'compact',
          description: `累计 ${usage.total_tokens} tokens / $${usage.total_cost_usd}${usage.favorite_model ? ' / ' + usage.favorite_model : ''}`,
        },
      ];
    }
    return [{ label: '查看用量并压缩', value: 'compact', description: '加载用量统计中...' }];
  }
  if (cmd === 'memory') return MEMORY_OPTIONS;
  if (cmd === 'review') return REVIEW_OPTIONS;
  if (cmd === 'init') return INIT_OPTIONS;
  return [];
});

// 对二级选项按查询词过滤
const filteredParamOptions = computed<SlashParamOption[]>(() => {
  const opts = currentParamOptions.value;
  const q = slashQuery.value.trim().toLowerCase();
  if (!q) return opts;
  return opts.filter(
    (o) => o.label.toLowerCase().includes(q) || (o.description?.toLowerCase().includes(q) ?? false),
  );
});

// 当前阶段显示的选项列表
const slashOptions = computed<SlashParamOption[]>(() => {
  if (slashPhase.value === 'command') {
    return filteredCommands.value.map((cmd) => ({
      label: `/${cmd.name}`,
      description: cmd.description,
      value: cmd.name,
    }));
  }
  if (slashPhase.value === 'param') {
    return filteredParamOptions.value;
  }
  return [];
});

// 解析当前输入，决定是否进入斜杠命令阶段
const refreshSlashState = () => {
  const text = currentInput.value;
  const el = textareaRef.value;

  // 非 / 开头则关闭
  if (!text.startsWith('/')) {
    slashPhase.value = null;
    slashActiveCommand.value = '';
    slashQuery.value = '';
    return;
  }

  // 找到第一个空格位置
  const firstSpace = text.indexOf(' ');
  const cursorPos = el?.selectionStart ?? text.length;

  // 命令名阶段：光标在第一个空格之前
  if (firstSpace === -1 || cursorPos <= firstSpace) {
    const name = text.slice(1, cursorPos);
    slashActiveCommand.value = '';
    slashPhase.value = 'command';
    slashQuery.value = name;
    slashSelectedIndex.value = 0;
    return;
  }

  // 已有空格：检查命令名是否匹配内置命令
  const cmdName = text.slice(1, firstSpace).toLowerCase();
  const matched = SLASH_COMMANDS.find((cmd) => cmd.name.toLowerCase() === cmdName);
  if (!matched) {
    slashPhase.value = null;
    return;
  }

  // 进入参数阶段：从第一个空格后到光标
  const argPart = text.slice(firstSpace + 1, cursorPos);
  // 参数中不能再有空格（单参数命令）
  if (argPart.includes(' ')) {
    slashPhase.value = null;
    return;
  }

  slashActiveCommand.value = matched.name;
  slashPhase.value = 'param';
  slashQuery.value = argPart;
  slashSelectedIndex.value = 0;
};

// 确保斜杠技能列表已加载
const ensureSlashSkillsLoaded = async () => {
  if (slashSkills.value.length === 0 && !slashSkillsLoading.value) {
    slashSkillsLoading.value = true;
    slashSkills.value = await loadSkills();
    slashSkillsLoading.value = false;
  }
};

// 加载用量统计（/compact 二级选项展示用）
const ensureUsageStatsLoaded = async () => {
  if (usageStats.value || usageLoading.value) return;
  usageLoading.value = true;
  try {
    const stats = await invoke<{ total_tokens: number; total_cost_usd: string; favorite_model?: string }>('get_usage_stats');
    usageStats.value = stats;
  } catch {
    // 加载失败不阻塞，仍显示压缩选项
    usageStats.value = { total_tokens: 0, total_cost_usd: '0' };
  } finally {
    usageLoading.value = false;
  }
};

const hideSlashMenu = () => {
  slashPhase.value = null;
  slashActiveCommand.value = '';
  slashQuery.value = '';
};

// 选择某个斜杠选项
const selectSlashOption = (option: SlashParamOption) => {
  if (slashPhase.value === 'command') {
    const entry = SLASH_COMMANDS.find((cmd) => cmd.name === option.value);
    if (!entry) return;

    // 进入参数阶段：命令名后强制加空格，光标定位到空格后
    currentInput.value = `/${option.value} `;
    nextTick(() => {
      const el = textareaRef.value;
      if (!el) return;
      const cmdEnd = `/${option.value} `.length;
      el.setSelectionRange(cmdEnd, cmdEnd);
      slashActiveCommand.value = option.value;
      slashPhase.value = 'param';
      slashQuery.value = '';
      slashSelectedIndex.value = 0;
      // 按命令类型预加载二级选项数据
      if (entry.type === 'skill') {
        void ensureSlashSkillsLoaded();
      } else if (entry.name === 'compact') {
        void ensureUsageStatsLoaded();
      }
    });
    return;
  }

  if (slashPhase.value === 'param') {
    // 参数阶段：选中二级选项后直接执行命令（参数为选项 value）
    const entry = SLASH_COMMANDS.find((cmd) => cmd.name === slashActiveCommand.value);
    if (!entry) return;
    hideSlashMenu();
    currentInput.value = '';
    void executeSlashCommand({ entry, rest: option.value });
    nextTick(() => {
      autoResize();
      focusTextarea();
    });
  }
};

// 斜杠菜单键盘导航：返回 true 表示已处理
const handleSlashKeydown = (e: KeyboardEvent): boolean => {
  if (slashPhase.value === null) return false;
  const opts = slashOptions.value;
  if (opts.length === 0) return false;

  if (e.key === 'ArrowDown') {
    e.preventDefault();
    slashSelectedIndex.value = (slashSelectedIndex.value + 1) % opts.length;
    return true;
  }
  if (e.key === 'ArrowUp') {
    e.preventDefault();
    slashSelectedIndex.value = (slashSelectedIndex.value - 1 + opts.length) % opts.length;
    return true;
  }
  if (e.key === 'Enter' || e.key === 'Tab') {
    e.preventDefault();
    const selected = opts[slashSelectedIndex.value];
    if (selected) selectSlashOption(selected);
    return true;
  }
  if (e.key === 'Escape') {
    e.preventDefault();
    hideSlashMenu();
    return true;
  }
  return false;
};

// 执行 Local 类型命令（不发送消息给 AI）。rest 为二级选项 value
const executeLocalCommand = async (entry: SlashCommandEntry, rest: string): Promise<boolean> => {
  if (entry.name === 'compact') {
    if (props.compacting) {
      emitToast({ message: '正在压缩中，请稍候' });
      return true;
    }
    emit('compact');
    return true;
  }
  if (entry.name === 'memory') {
    if (rest === 'clear') {
      try {
        await invoke('clear_memory_entries');
        memoryEntries.value = [];
        emitToast({ message: '全局记忆已清空' });
      } catch (error) {
        emitErrorToast('清空记忆', error);
      }
      return true;
    }
    // 默认 view：展示全局记忆
    memoryViewOpen.value = true;
    if (memoryEntries.value.length === 0 && !memoryLoading.value) {
      memoryLoading.value = true;
      try {
        memoryEntries.value = await invoke<string[]>('list_memory_entries');
      } catch (error) {
        emitErrorToast('加载记忆', error);
        memoryViewOpen.value = false;
      } finally {
        memoryLoading.value = false;
      }
    }
    return true;
  }
  return false;
};

// 执行 Prompt 类型命令（构造模板消息发送给 AI）。rest 为二级选项 value
const executePromptCommand = async (entry: SlashCommandEntry, rest: string): Promise<boolean> => {
  if (entry.name === 'init') {
    emit('send', buildInitPrompt(rest));
    return true;
  }
  if (entry.name === 'review') {
    if (reviewLoading.value) return true;
    reviewLoading.value = true;
    try {
      const diff = await getWorkspaceDiff(null);
      const diffText = formatWorkspaceDiff(diff);
      const scope = rest === 'all' ? '（含未跟踪文件）' : '（已跟踪改动）';
      const prompt = `${buildReviewPrompt(scope)}\n\n## 工作区 diff\n\n\`\`\`diff\n${diffText}\n\`\`\``;
      emit('send', prompt);
    } catch (error) {
      emitErrorToast('获取工作区改动', error);
    } finally {
      reviewLoading.value = false;
    }
    return true;
  }
  return false;
};

// 执行已识别的斜杠命令。返回 true 表示已处理（应清空输入框）
const executeSlashCommand = async (parsed: { entry: SlashCommandEntry; rest: string }): Promise<boolean> => {
  const { entry, rest } = parsed;
  if (entry.type === 'local') {
    return executeLocalCommand(entry, rest);
  }
  if (entry.type === 'prompt') {
    return executePromptCommand(entry, rest);
  }
  if (entry.type === 'skill') {
    // rest 为技能名
    if (!rest) return false;
    emit('send', `请使用 Skill 工具加载并执行技能：${rest}`);
    return true;
  }
  return false;
};

// 转义 HTML 特殊字符，防止镜像层渲染用户输入时出现 XSS 或解析错误
const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

// 输入框镜像层内容：高亮开头的斜杠命令名（如 /skill）
const highlightedInput = computed(() => {
  const text = currentInput.value;
  if (!text) return '';
  const escaped = escapeHtml(text);
  // 匹配开头的 /命令名（字母开头，可含连字符）
  const match = escaped.match(/^(\/[a-zA-Z][\w-]*)/);
  if (match) {
    const cmd = match[1];
    const rest = escaped.slice(cmd.length);
    return `<span class="text-primary font-semibold">${cmd}</span>${rest}`;
  }
  return escaped;
});

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

const loadImageElement = (dataUrl: string): Promise<HTMLImageElement> =>
  new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error('图片解码失败'));
    img.src = dataUrl;
  });

const canvasToDataUrl = (
  img: HTMLImageElement,
  width: number,
  height: number,
  mime: string,
  quality?: number,
): string => {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Canvas 2D 上下文不可用');
  ctx.drawImage(img, 0, 0, width, height);
  return quality !== undefined ? canvas.toDataURL(mime, quality) : canvas.toDataURL(mime);
};

const base64ByteLength = (dataUrl: string): number => {
  const commaIndex = dataUrl.indexOf(',');
  if (commaIndex < 0) return 0;
  const base64 = dataUrl.slice(commaIndex + 1);
  const padding = base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0;
  return Math.floor((base64.length * 3) / 4) - padding;
};

// 缩放图片到符合 MAX_IMAGE_BASE64_BYTES 限制；保持原 mime（gif 不缩放，直接返回原 dataUrl）
const resizeImageIfNeeded = async (
  dataUrl: string,
  mimeType: string,
): Promise<{ dataUrl: string; mimeType: string }> => {
  if (mimeType === 'image/gif') {
    return { dataUrl, mimeType };
  }

  const originalBytes = base64ByteLength(dataUrl);
  if (originalBytes <= MAX_IMAGE_BASE64_BYTES) {
    return { dataUrl, mimeType };
  }

  const img = await loadImageElement(dataUrl);
  const originalWidth = img.naturalWidth;
  const originalHeight = img.naturalHeight;

  // 计算初始缩放比例：先限制最大尺寸，再逐级 ×0.75 降采样
  const initialScale = Math.min(
    1,
    IMAGE_MAX_DIMENSION / originalWidth,
    IMAGE_MAX_DIMENSION / originalHeight,
  );
  let currentWidth = Math.max(1, Math.round(originalWidth * initialScale));
  let currentHeight = Math.max(1, Math.round(originalHeight * initialScale));

  // 尝试当前尺寸 + 逐级降采样，每级尝试 PNG（无损）+ 多档 JPEG
  while (currentWidth >= 1 && currentHeight >= 1) {
    const encoders: Array<{ mime: string; quality?: number }> = [
      { mime: mimeType },
      ...IMAGE_JPEG_QUALITIES.map((q) => ({ mime: 'image/jpeg', quality: q })),
    ];
    for (const encoder of encoders) {
      const candidate = canvasToDataUrl(img, currentWidth, currentHeight, encoder.mime, encoder.quality);
      if (base64ByteLength(candidate) <= MAX_IMAGE_BASE64_BYTES) {
        return { dataUrl: candidate, mimeType: encoder.mime };
      }
    }
    if (currentWidth === 1 && currentHeight === 1) break;
    currentWidth = Math.max(1, Math.floor(currentWidth * IMAGE_RESIZE_SCALE_STEPS));
    currentHeight = Math.max(1, Math.floor(currentHeight * IMAGE_RESIZE_SCALE_STEPS));
  }

  throw new Error(`图片缩放后仍超过 ${Math.round(MAX_IMAGE_BASE64_BYTES / 1024 / 1024)}MB 限制`);
};

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
      if (file.size > MAX_UPLOAD_FILE_SIZE_BYTES) {
        rejected.push(`${file.name || `图片${i + 1}`}: 超过 100MB 限制`);
        continue;
      }

      let dataUrl: string;
      try {
        dataUrl = await readAsDataUrl(file);
      } catch {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片读取失败`);
        continue;
      }

      let finalMimeType = imageMimeType;
      try {
        const result = await resizeImageIfNeeded(dataUrl, imageMimeType);
        dataUrl = result.dataUrl;
        finalMimeType = result.mimeType;
      } catch (error) {
        const message = error instanceof Error ? error.message : '图片缩放失败';
        rejected.push(`${file.name || `图片${i + 1}`}: ${message}`);
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
        sourceName: file.name || fallbackPastedImageName(finalMimeType, i),
        mimeType: finalMimeType,
        mediaType: finalMimeType,
        data: base64Data,
        size: base64ByteLength(dataUrl),
      };
      accepted.push(imageItem);
      continue;
    }

    if (file.size > MAX_UPLOAD_FILE_SIZE_BYTES) {
      rejected.push(`${file.name || `文件${i + 1}`}: 超过 100MB 限制`);
      continue;
    }

    const ext = extensionOf(file.name);
    const isBinaryDoc = ext === 'docx' || ext === 'pptx' || ext === 'pdf';

    if (isBinaryDoc) {
      let rawBytes: number[];
      try {
        const buf = await file.arrayBuffer();
        rawBytes = Array.from(new Uint8Array(buf));
      } catch {
        rejected.push(`${file.name || `文件${i + 1}`}: 文件读取失败`);
        continue;
      }

      let content: string | null = null;
      if (ext === 'docx' || ext === 'pptx') {
        try {
          const parsed = await parseDocumentUploadFile(file);
          content = parsed.content;
        } catch (error) {
          const message = error instanceof Error ? error.message : '文件解析失败';
          rejected.push(`${file.name || `文件${i + 1}`}: ${message}`);
          continue;
        }
      }

      const textItem: UploadedDocumentFile = {
        kind: 'document',
        sourceName: file.name,
        mimeType: file.type || undefined,
        content,
        rawBytes,
        size: file.size,
      };
      accepted.push(textItem);
      continue;
    }

    // 纯文本类文件：直接读取内容，注入对话上下文
    let textContent: string;
    try {
      textContent = await file.text();
    } catch {
      rejected.push(`${file.name || `文件${i + 1}`}: 文件读取失败`);
      continue;
    }

    const textItem: UploadedDocumentFile = {
      kind: 'document',
      sourceName: file.name,
      mimeType: file.type || undefined,
      content: textContent,
      rawBytes: null,
      size: file.size,
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

// textarea 输入事件：先调整高度，再刷新斜杠命令状态
const onTextareaInput = () => {
  autoResize();
  refreshSlashState();
};

// textarea keydown 事件：斜杠菜单激活时优先拦截导航键，否则交给 sendMessage
const onTextareaKeydown = (e: KeyboardEvent) => {
  if (slashPhase.value !== null) {
    if (handleSlashKeydown(e)) return;
    // 斜杠菜单激活时，Enter 已被 handleSlashKeydown 处理；若未处理则放行
  }
  if (e.key === 'Enter' && !e.shiftKey) {
    sendMessage(e);
  }
};


// 发送消息，支持 Shift + Enter 换行，当 isGenerating 为 true 时禁用发送功能
const sendMessage = (e?: KeyboardEvent) => {
  if (e && e.shiftKey) return;
  e?.preventDefault();
  if ((!currentInput.value.trim() && !hasPendingUploads.value) || props.isGenerating) return;

  const trimmed = currentInput.value.trim();

  // 识别斜杠命令：所有命令都需通过二级选项选择参数后执行
  const parsed = parseSlashCommand(trimmed);
  if (parsed) {
    const { entry, rest } = parsed;
    // options 类型命令必须带参数（通过二级选项填入）；无参时回车不执行
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
  currentInput.value = "";
  hideSlashMenu();
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

// 点击浮层外部时关闭 + 菜单和 memory 浮层
const handleDocumentClick = (e: MouseEvent) => {
  const target = e.target as Node | null;
  // 关闭 + 菜单
  if (plusMenuView.value !== null) {
    if (plusButtonRef.value && target && plusButtonRef.value.contains(target)) return;
    const menus = document.querySelectorAll('[data-plus-menu]');
    for (const menu of menus) {
      if (menu.contains(target)) return;
    }
    closePlusMenu();
  }
  // 关闭 memory 浮层
  if (memoryViewOpen.value) {
    const memMenu = document.querySelector('[data-memory-menu]');
    if (memMenu && target && memMenu.contains(target)) return;
    memoryViewOpen.value = false;
  }
};

onMounted(() => {
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
            <span
              v-if="file.kind === 'document'"
              class="rounded-md bg-black/5 px-1.5 py-0.5 text-[10px] leading-none text-[#64748b] dark:bg-white/10 dark:text-[#cbd5e1]"
              title="上传的文件将保存为会话文件，AI 可通过 Read 工具随时读取。"
            >
              会话文件
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
      <div class="relative w-full">
        <!-- 高亮镜像层：显示带命令高亮的输入内容，位于 textarea 下方 -->
        <div
          aria-hidden="true"
          class="absolute inset-0 w-full px-4 pt-3 pb-2 text-[0.95rem] text-[#1a1a1a] dark:text-[#ececec] max-h-[40vh] overflow-hidden whitespace-pre-wrap break-words pointer-events-none"
          v-html="highlightedInput + '\u200b'"></div>
        <textarea ref="textareaRef" v-model="currentInput" @keydown="onTextareaKeydown" @input="onTextareaInput" @paste="onTextareaPaste" @compositionstart="isComposing = true" @compositionend="isComposing = false"
          placeholder="Message Nova..." rows="1"
          :class="['relative w-full bg-transparent border-none text-[0.95rem] caret-[#1a1a1a] dark:caret-[#ececec] resize-none outline-none block max-h-[40vh] px-4 pt-3 pb-2 placeholder:text-[#a3a3a3] z-10', isComposing ? 'text-[#1a1a1a] dark:text-[#ececec]' : 'text-transparent']"></textarea>

        <!-- 斜杠命令下拉菜单：向上弹出，与输入框同宽 -->
        <div
          v-if="slashPhase !== null && slashOptions.length > 0"
          class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden">
          <div class="max-h-[240px] overflow-y-auto py-1">
            <button
              v-for="(option, index) in slashOptions"
              :key="option.label + index"
              type="button"
              class="w-full flex items-center justify-between gap-2 px-3 py-1.5 text-left transition-colors"
              :class="{ 'bg-secondary/80': index === slashSelectedIndex }"
              @mouseenter="slashSelectedIndex = index"
              @click="selectSlashOption(option)">
              <div class="flex items-center gap-2 min-w-0">
                <span class="text-sm font-medium truncate">{{ option.label }}</span>
              </div>
              <span v-if="option.description" class="text-xs text-muted-foreground truncate shrink-0 max-w-[60%]">{{ option.description }}</span>
            </button>
          </div>
        </div>
        <div
          v-else-if="slashPhase === 'param' && slashSkillsLoading"
          class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden">
          <div class="px-3 py-2 text-xs text-muted-foreground">加载中...</div>
        </div>
        <div
          v-else-if="slashPhase === 'param' && slashOptions.length === 0 && !slashSkillsLoading"
          class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden">
          <div class="px-3 py-2 text-xs text-muted-foreground">暂无匹配项</div>
        </div>

        <!-- /memory 浮层：展示全局记忆条目（与输入框同宽） -->
        <div
          v-if="memoryViewOpen"
          data-memory-menu
          class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden">
          <div class="flex items-center justify-between gap-2 px-3 py-2 border-b border-border">
            <span class="text-xs font-medium text-muted-foreground">全局记忆</span>
            <button
              type="button"
              class="shrink-0 rounded p-0.5 hover:bg-secondary/80 transition-colors"
              @click="memoryViewOpen = false">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                stroke-linecap="round" stroke-linejoin="round">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>
          <div class="max-h-[240px] overflow-y-auto">
            <div v-if="memoryLoading" class="px-3 py-2 text-xs text-muted-foreground">加载中...</div>
            <div v-else-if="memoryEntries.length === 0" class="px-3 py-2 text-xs text-muted-foreground">暂无记忆条目</div>
            <div
              v-for="(entry, idx) in memoryEntries"
              :key="idx"
              class="px-3 py-2 text-sm border-b border-border/50 last:border-b-0 whitespace-pre-wrap break-words">
              {{ entry }}
            </div>
          </div>
        </div>

        <!-- + 按钮菜单：主视图（与输入框同宽） -->
        <div
          v-if="plusMenuView === 'main'"
          data-plus-menu
          class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden">
          <button
            type="button"
            class="w-full flex items-center gap-2 px-3 py-2 text-sm text-left hover:bg-secondary/80 transition-colors"
            @click="pickUploadFromPlusMenu">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
              stroke-linecap="round" stroke-linejoin="round" class="shrink-0">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12" />
            </svg>
            <span>上传文件</span>
          </button>
          <button
            type="button"
            class="w-full flex items-center justify-between gap-2 px-3 py-2 text-sm text-left hover:bg-secondary/80 transition-colors"
            @click="enterSkillView">
            <div class="flex items-center gap-2">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                stroke-linecap="round" stroke-linejoin="round" class="shrink-0">
                <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
              </svg>
              <span>使用技能</span>
            </div>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
              stroke-linecap="round" stroke-linejoin="round" class="shrink-0 opacity-60">
              <path d="M9 18l6-6-6-6" />
            </svg>
          </button>
        </div>

        <!-- + 按钮菜单：技能视图（与输入框同宽） -->
        <div
          v-if="plusMenuView === 'skill'"
          data-plus-menu
          class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden">
          <div class="flex items-center gap-2 px-3 py-2 border-b border-border">
            <button
              type="button"
              class="shrink-0 rounded p-0.5 hover:bg-secondary/80 transition-colors"
              @click="plusMenuView = 'main'">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                stroke-linecap="round" stroke-linejoin="round">
                <path d="M15 18l-6-6 6-6" />
              </svg>
            </button>
            <span class="text-xs font-medium text-muted-foreground">技能列表</span>
          </div>
          <div class="max-h-[240px] overflow-y-auto">
            <div v-if="skillsLoading" class="px-3 py-2 text-xs text-muted-foreground">加载中...</div>
            <div v-else-if="skills.length === 0" class="px-3 py-2 text-xs text-muted-foreground">暂无可用技能</div>
            <button
              v-for="skill in skills"
              :key="skill.name"
              type="button"
              class="w-full flex flex-col items-start gap-0.5 px-3 py-1.5 text-left hover:bg-secondary/80 transition-colors"
              @click="pickSkillFromPlusMenu(skill)">
              <span class="text-sm truncate w-full">{{ skill.name }}</span>
              <span v-if="skill.description" class="text-xs text-muted-foreground truncate w-full">{{ skill.description }}</span>
            </button>
          </div>
        </div>
      </div>

      <div class="flex min-w-0 items-center gap-2 px-3 pb-3 pt-2">
        <div class="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <button
            ref="plusButtonRef"
            type="button"
            class="w-8 h-8 shrink-0 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-secondary/80 transition-colors"
            :class="{ 'bg-secondary/80': plusMenuView !== null }"
            @click="plusMenuView !== null ? closePlusMenu() : openPlusMenu()">
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
            <ContextUsageIndicator :usage="contextUsage" :usedTokens="contextTokens" :model="currentModel" :compacting="compacting" @compact="emit('compact')" />
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
