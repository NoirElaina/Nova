<script setup lang="ts">
import { computed } from 'vue'
import { Check, Cpu, Edit, Trash2 } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'

const props = defineProps<{
  id: string
  label: string
  apiFormat: string
  model: string
  isCurrent: boolean
}>()

const emit = defineEmits<{
  (e: 'switch', id: string): void
  (e: 'edit', id: string): void
  (e: 'delete', id: string): void
}>()

const logoText = computed(() => {
  return props.label.substring(0, 2).toUpperCase()
})
</script>

<template>
  <div
    :class="[
      'group relative flex items-center gap-4 rounded-xl border bg-card p-4 transition-all hover:shadow-md',
      isCurrent ? 'border-primary ring-1 ring-primary/20' : 'border-border'
    ]"
  >
    <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-muted text-lg font-semibold text-muted-foreground">
      {{ logoText }}
    </div>

    <div class="flex min-w-0 flex-1 flex-col gap-1">
      <div class="flex items-center gap-2">
        <span class="truncate font-semibold text-foreground">{{ label }}</span>
        <span v-if="isCurrent" class="flex items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
          <Check class="h-3 w-3" /> 当前使用
        </span>
      </div>
      <div class="flex items-center gap-2 text-xs text-muted-foreground">
        <Cpu class="h-3 w-3" />
        <span class="truncate">{{ model || '未配置模型' }}</span>
        <span>•</span>
        <span class="uppercase">{{ apiFormat.replace('_', ' ') }}</span>
      </div>
    </div>

    <div class="flex shrink-0 items-center gap-2">
      <Button
        variant="secondary"
        size="sm"
        :disabled="isCurrent"
        @click="emit('switch', id)"
      >
        {{ isCurrent ? '使用中' : '使用' }}
      </Button>

      <Button variant="ghost" size="icon" class="h-8 w-8 text-muted-foreground hover:bg-muted" @click="emit('edit', id)">
        <Edit class="h-4 w-4" />
      </Button>
      <Button variant="ghost" size="icon" class="h-8 w-8 text-destructive hover:bg-destructive/10" @click="emit('delete', id)">
        <Trash2 class="h-4 w-4" />
      </Button>
    </div>
  </div>
</template>
