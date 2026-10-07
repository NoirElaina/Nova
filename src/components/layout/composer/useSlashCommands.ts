import { ref, computed, nextTick, type Ref, type ComputedRef } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { emitToast, emitErrorToast } from '../../../lib/toast';
import { getWorkspaceDiff } from '../../../features/chat/services/chat-api';
import {
  MEMORY_OPTIONS,
  REVIEW_OPTIONS,
  INIT_OPTIONS,
  AGENT_OPTIONS,
  SKILL_CREATE_VALUE,
  buildInitPrompt,
  buildReviewPrompt,
  buildCreateAgentPrompt,
  buildCreateSkillPrompt,
  formatWorkspaceDiff,
  allSlashCommands,
  type SlashCommandEntry,
  type SlashParamOption,
} from '../../../lib/slash-commands';

export interface SkillSummary {
  name: string;
  description: string;
  path: string;
}

export interface UseSlashCommandsOptions {
  currentInput: Ref<string>;
  textareaRef: Ref<HTMLTextAreaElement | null>;
  conversationId: ComputedRef<string | null | undefined>;
  compacting: ComputedRef<boolean>;
  onSend: (msg: string) => void;
  onCompact: () => void;
  autoResize: () => void;
  focusTextarea: () => void;
}

export function useSlashCommands(options: UseSlashCommandsOptions) {
  const {
    currentInput,
    textareaRef,
    conversationId: _conversationId,
    compacting,
    onSend,
    onCompact,
    autoResize,
    focusTextarea,
  } = options;

  const slashPhase = ref<null | 'command' | 'param'>(null);
  const slashQuery = ref('');
  const slashSelectedIndex = ref(0);
  const slashSkills = ref<SkillSummary[]>([]);
  const slashSkillsLoading = ref(false);
  const slashActiveCommand = ref<string>('');

  const memoryEntries = ref<string[]>([]);
  const memoryLoading = ref(false);
  const memoryViewOpen = ref(false);

  const reviewLoading = ref(false);

  const usageStats = ref<{ total_tokens: number; total_cost_usd: string; favorite_model?: string } | null>(null);
  const usageLoading = ref(false);

  const loadSkills = async (): Promise<SkillSummary[]> => {
    try {
      const list = await invoke<SkillSummary[]>('list_skills');
      return list || [];
    } catch (error) {
      console.error('Failed to load skills:', error);
      return [];
    }
  };

  const filteredCommands = computed(() => {
    const q = slashQuery.value.trim().toLowerCase();
    const commands = allSlashCommands();
    if (!q) return commands;
    return commands.filter((cmd) => cmd.name.toLowerCase().includes(q));
  });

  const currentParamOptions = computed<SlashParamOption[]>(() => {
    const cmd = slashActiveCommand.value;
    if (cmd === 'skill') {
      return [
        ...slashSkills.value.map((s) => ({
          label: s.name,
          description: s.description,
          value: s.name,
        })),
        {
          label: '＋ 创建新技能',
          value: SKILL_CREATE_VALUE,
          description: '让 AI 采访需求并编写新的 SKILL.md',
        },
      ];
    }
    if (cmd === 'compact') {
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
    if (cmd === 'agent') return AGENT_OPTIONS;

    return [];
  });

  const filteredParamOptions = computed<SlashParamOption[]>(() => {
    const opts = currentParamOptions.value;
    const q = slashQuery.value.trim().toLowerCase();
    if (!q) return opts;
    return opts.filter(
      (o) => o.label.toLowerCase().includes(q) || (o.description?.toLowerCase().includes(q) ?? false),
    );
  });

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

  const refreshSlashState = () => {
    const text = currentInput.value;
    const el = textareaRef.value;

    if (!text.startsWith('/')) {
      slashPhase.value = null;
      slashActiveCommand.value = '';
      slashQuery.value = '';
      return;
    }

    const firstSpace = text.indexOf(' ');
    const cursorPos = el?.selectionStart ?? text.length;

    if (firstSpace === -1 || cursorPos <= firstSpace) {
      const name = text.slice(1, cursorPos);
      slashActiveCommand.value = '';
      slashPhase.value = 'command';
      slashQuery.value = name;
      slashSelectedIndex.value = 0;
      return;
    }

    const cmdName = text.slice(1, firstSpace).toLowerCase();
    const matched = allSlashCommands().find((cmd) => cmd.name.toLowerCase() === cmdName);
    if (!matched) {
      slashPhase.value = null;
      return;
    }

    const argPart = text.slice(firstSpace + 1, cursorPos);
    if (argPart.includes(' ')) {
      slashPhase.value = null;
      return;
    }

    slashActiveCommand.value = matched.name;
    slashPhase.value = 'param';
    slashQuery.value = argPart;
    slashSelectedIndex.value = 0;
  };

  const ensureSlashSkillsLoaded = async () => {
    if (slashSkills.value.length === 0 && !slashSkillsLoading.value) {
      slashSkillsLoading.value = true;
      slashSkills.value = await loadSkills();
      slashSkillsLoading.value = false;
    }
  };

  const ensureUsageStatsLoaded = async () => {
    if (usageStats.value || usageLoading.value) return;
    usageLoading.value = true;
    try {
      const stats = await invoke<{ total_tokens: number; total_cost_usd: string; favorite_model?: string }>('get_usage_stats');
      usageStats.value = stats;
    } catch {
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

  const fetchAppDataDir = async (): Promise<string | null> => {
    try {
      return await invoke<string>('get_app_data_dir');
    } catch (error) {
      emitErrorToast('获取应用数据目录', error);
      return null;
    }
  };

  const executeLocalCommand = async (entry: SlashCommandEntry, rest: string): Promise<boolean> => {
    if (entry.name === 'compact') {
      if (compacting.value) {
        emitToast({ message: '正在压缩中，请稍候' });
        return true;
      }
      onCompact();
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

  const executePromptCommand = async (entry: SlashCommandEntry, rest: string): Promise<boolean> => {
    if (entry.name === 'init') {
      onSend(buildInitPrompt(rest));
      return true;
    }
    if (entry.name === 'agent') {
      const appDataDir = await fetchAppDataDir();
      if (!appDataDir) return true;
      onSend(buildCreateAgentPrompt(appDataDir, rest));
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
        onSend(prompt);
      } catch (error) {
        emitErrorToast('获取工作区改动', error);
      } finally {
        reviewLoading.value = false;
      }
      return true;
    }
    return false;
  };

  const executeSlashCommand = async (parsed: { entry: SlashCommandEntry; rest: string }): Promise<boolean> => {
    const { entry, rest } = parsed;
    if (entry.type === 'local') {
      return executeLocalCommand(entry, rest);
    }
    if (entry.type === 'prompt') {
      return executePromptCommand(entry, rest);
    }
    if (entry.type === 'skill') {
      if (!rest) return false;
      if (rest === SKILL_CREATE_VALUE) {
        const appDataDir = await fetchAppDataDir();
        if (!appDataDir) return true;
        onSend(buildCreateSkillPrompt(appDataDir));
        return true;
      }
      onSend(`请使用 Skill 工具加载并执行技能：${rest}`);
      return true;
    }
    return false;
  };

  const selectSlashOption = (option: SlashParamOption) => {
    if (slashPhase.value === 'command') {
      const entry = allSlashCommands().find((cmd) => cmd.name === option.value);
      if (!entry) return;

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
        if (entry.type === 'skill') {
          void ensureSlashSkillsLoaded();
        } else if (entry.name === 'compact') {
          void ensureUsageStatsLoaded();
        }
      });
      return;
    }

    if (slashPhase.value === 'param') {
      const entry = allSlashCommands().find((cmd) => cmd.name === slashActiveCommand.value);
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

  return {
    slashPhase,
    slashQuery,
    slashSelectedIndex,
    slashSkills,
    slashSkillsLoading,
    slashActiveCommand,
    slashOptions,
    memoryEntries,
    memoryLoading,
    memoryViewOpen,
    reviewLoading,
    loadSkills,
    refreshSlashState,
    hideSlashMenu,
    selectSlashOption,
    handleSlashKeydown,
    executeSlashCommand,
  };
}
