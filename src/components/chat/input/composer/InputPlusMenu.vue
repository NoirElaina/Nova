<script setup lang="ts">
import type { SkillSummary } from './useSlashCommands';

defineProps<{
  view: 'main' | 'skill' | null;
  skills: SkillSummary[];
  loading: boolean;
}>();

const emit = defineEmits<{
  (e: 'upload'): void;
  (e: 'enter-skill'): void;
  (e: 'back-main'): void;
  (e: 'select-skill', skill: SkillSummary): void;
}>();
</script>

<template>
  <div>
    <!-- + 按钮菜单：主视图（与输入框同宽） -->
    <div
      v-if="view === 'main'"
      data-plus-menu
      class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden"
    >
      <button
        type="button"
        class="w-full flex items-center gap-2 px-3 py-2 text-sm text-left hover:bg-secondary/80 transition-colors"
        @click="emit('upload')"
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="shrink-0"
        >
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12" />
        </svg>
        <span>上传文件</span>
      </button>
      <button
        type="button"
        class="w-full flex items-center justify-between gap-2 px-3 py-2 text-sm text-left hover:bg-secondary/80 transition-colors"
        @click="emit('enter-skill')"
      >
        <div class="flex items-center gap-2">
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class="shrink-0"
          >
            <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
          </svg>
          <span>使用技能</span>
        </div>
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="shrink-0 opacity-60"
        >
          <path d="M9 18l6-6-6-6" />
        </svg>
      </button>
    </div>

    <!-- + 按钮菜单：技能视图（与输入框同宽） -->
    <div
      v-if="view === 'skill'"
      data-plus-menu
      class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden"
    >
      <div class="flex items-center gap-2 px-3 py-2 border-b border-border">
        <button
          type="button"
          class="shrink-0 rounded p-0.5 hover:bg-secondary/80 transition-colors"
          @click="emit('back-main')"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M15 18l-6-6 6-6" />
          </svg>
        </button>
        <span class="text-xs font-medium text-muted-foreground">技能列表</span>
      </div>
      <div class="max-h-[240px] overflow-y-auto">
        <div v-if="loading" class="px-3 py-2 text-xs text-muted-foreground">加载中...</div>
        <div v-else-if="skills.length === 0" class="px-3 py-2 text-xs text-muted-foreground">暂无可用技能</div>
        <button
          v-for="skill in skills"
          :key="skill.name"
          type="button"
          class="w-full flex flex-col items-start gap-0.5 px-3 py-1.5 text-left hover:bg-secondary/80 transition-colors"
          @click="emit('select-skill', skill)"
        >
          <span class="text-sm truncate w-full">{{ skill.name }}</span>
          <span v-if="skill.description" class="text-xs text-muted-foreground truncate w-full">{{ skill.description }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
