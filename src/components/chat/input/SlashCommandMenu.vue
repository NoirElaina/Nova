<script setup lang="ts">
import type { SlashParamOption } from '@/lib/slash-commands';

defineProps<{
  visible: boolean;
  options: SlashParamOption[];
  selectedIndex: number;
  loading?: boolean;
}>();

const emit = defineEmits<{
  (e: 'select', option: SlashParamOption): void;
  (e: 'update:selectedIndex', index: number): void;
}>();
</script>

<template>
  <div
    v-if="visible"
    class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden flex flex-col max-h-[260px] custom-scrollbar overflow-y-auto p-1"
  >
    <div v-if="loading" class="flex items-center justify-center py-3 text-xs text-muted-foreground">
      <svg class="animate-spin h-3.5 w-3.5 mr-2" viewBox="0 0 24 24" fill="none">
        <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
        <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
      </svg>
      加载中...
    </div>
    <div v-else-if="options.length === 0" class="py-3 text-center text-xs text-muted-foreground">
      无匹配命令或选项
    </div>
    <button
      v-else
      v-for="(option, index) in options"
      :key="`${option.value}-${option.label}-${index}`"
      type="button"
      class="w-full flex flex-col items-start px-3 py-1.5 rounded-md text-left transition-colors"
      :class="index === selectedIndex ? 'bg-secondary text-foreground' : 'hover:bg-secondary/60 text-muted-foreground hover:text-foreground'"
      @mouseenter="emit('update:selectedIndex', index)"
      @click="emit('select', option)"
    >
      <span class="text-xs font-medium text-foreground truncate w-full">{{ option.label }}</span>
      <span v-if="option.description" class="text-[11px] text-muted-foreground truncate w-full">
        {{ option.description }}
      </span>
    </button>
  </div>
</template>
