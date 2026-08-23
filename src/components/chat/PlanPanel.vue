<script setup lang="ts">
import { Button } from '@/components/ui/button';
import { ListChecks, X } from 'lucide-vue-next';
import PlanTab from './workspace/PlanTab.vue';

defineProps<{
  open: boolean;
  conversationId: string | null;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
}>();
</script>

<template>
  <Transition name="slide-right">
    <aside v-show="open" class="relative z-20 h-full w-[440px] shrink-0 py-2 pl-1.5">
      <!-- 悬浮圆角卡片：与工作区抽屉同款外观 -->
      <div
        class="flex h-full flex-col overflow-hidden rounded-2xl border border-[#e7e9ee] bg-white shadow-[0_8px_30px_rgba(15,23,42,0.08)] dark:border-[#343434] dark:bg-[#1e1e1e] dark:shadow-[0_8px_30px_rgba(0,0,0,0.4)]"
      >
        <div class="flex h-11 shrink-0 items-center justify-between border-b border-[#eef0f3] px-3 dark:border-[#2c2c2c]">
          <div class="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-[#111827] dark:text-[#ececec]">
            <ListChecks class="h-3.5 w-3.5 shrink-0 text-[#64748b] dark:text-[#9ca3af]" />
            执行计划
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            class="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-black/5 dark:hover:bg-white/5"
            title="关闭计划面板"
            @click="emit('close')"
          >
            <X class="h-4 w-4" />
          </Button>
        </div>
        <div class="min-h-0 flex-1 overflow-hidden">
          <PlanTab :conversationId="conversationId" />
        </div>
      </div>
    </aside>
  </Transition>
</template>

<style scoped>
.slide-right-enter-active,
.slide-right-leave-active {
  transition:
    width 0.24s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.2s ease;
}

.slide-right-enter-from,
.slide-right-leave-to {
  width: 0 !important;
  opacity: 0;
}

.slide-right-enter-to,
.slide-right-leave-from {
  opacity: 1;
}
</style>
