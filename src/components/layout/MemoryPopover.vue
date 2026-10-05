<script setup lang="ts">
defineProps<{
  open: boolean;
  loading: boolean;
  entries: string[];
}>();

const emit = defineEmits<{
  (e: 'close'): void;
}>();
</script>

<template>
  <div
    v-if="open"
    data-memory-menu
    class="absolute bottom-full left-0 mb-2 w-full rounded-lg border border-border bg-popover shadow-lg z-50 overflow-hidden flex flex-col max-h-[320px]"
  >
    <div class="flex items-center justify-between px-3 py-2 border-b border-border text-xs font-medium text-muted-foreground">
      <span>全局记忆条目</span>
      <button
        type="button"
        class="text-xs hover:text-foreground transition-colors"
        @click="emit('close')"
      >
        关闭
      </button>
    </div>

    <div class="overflow-y-auto flex-1 p-2 space-y-1.5 custom-scrollbar">
      <div v-if="loading" class="flex items-center justify-center py-4 text-xs text-muted-foreground">
        <svg class="animate-spin h-4 w-4 mr-2" viewBox="0 0 24 24" fill="none">
          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
          <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
        </svg>
        加载记忆中...
      </div>
      <div v-else-if="entries.length === 0" class="py-4 text-center text-xs text-muted-foreground">
        暂无全局记忆条目
      </div>
      <div
        v-else
        v-for="(entry, index) in entries"
        :key="index"
        class="text-xs p-2 rounded bg-secondary/50 border border-border/50 text-foreground break-words"
      >
        {{ entry }}
      </div>
    </div>
  </div>
</template>
