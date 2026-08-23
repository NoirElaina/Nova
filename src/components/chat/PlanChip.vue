<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { ListChecks } from 'lucide-vue-next';
import { getConversationPlan } from '../../features/chat/services/chat-api';

const props = defineProps<{
  conversationId?: string | null;
}>();

const emit = defineEmits<{
  (e: 'open'): void;
}>();

/** 计划标题（计划内容首行去掉 markdown 标记）；为空时小框不显示。 */
const planTitle = ref('');

const extractTitle = (content: string) => {
  const line = content.trim().split(/\r?\n/).find((item) => item.trim());
  return line ? line.replace(/^#+\s*/, '').trim() : '';
};

const loadPlan = async () => {
  const target = props.conversationId;
  try {
    const plan = await getConversationPlan(target ?? null);
    // 会话可能已切走，避免旧请求覆盖新会话数据。
    if (target !== props.conversationId) return;
    planTitle.value = extractTitle(plan?.content ?? '');
  } catch {
    if (target === props.conversationId) {
      planTitle.value = '';
    }
  }
};

watch(
  () => props.conversationId,
  () => {
    planTitle.value = '';
    void loadPlan();
  },
  { immediate: true },
);

let unlistenPlanUpdated: UnlistenFn | null = null;

onMounted(() => {
  listen<{ conversationId?: string | null; content?: string }>('plan-updated', (event) => {
    const payload = event.payload;
    const current = props.conversationId;
    if (current && payload.conversationId && payload.conversationId !== current) {
      return;
    }
    if (typeof payload.content === 'string') {
      planTitle.value = extractTitle(payload.content);
    } else {
      void loadPlan();
    }
  })
    .then((unlisten) => {
      unlistenPlanUpdated = unlisten;
    })
    .catch((error) => {
      console.warn('Plan chip listener failed:', error);
    });
});

onBeforeUnmount(() => {
  unlistenPlanUpdated?.();
});
</script>

<template>
  <!-- 输入框上方的计划小框：有计划才显示，点击打开计划侧边面板。
       尺寸与输入框工具栏（盾牌/模型下拉）对齐：紧凑单行。 -->
  <button
    v-if="planTitle"
    type="button"
    class="inline-flex h-7 max-w-full items-center gap-1.5 rounded-lg border border-[#e7e9ee] bg-[#fafbfc] px-2 text-left transition-colors hover:bg-[#f3f5f8] dark:border-[#343434] dark:bg-white/5 dark:hover:bg-white/10"
    title="查看执行计划"
    @click="emit('open')"
  >
    <ListChecks class="h-3.5 w-3.5 shrink-0 text-[#64748b] dark:text-[#9ca3af]" />
    <span class="shrink-0 text-[11px] text-[#8a94a3] dark:text-[#858585]">执行计划</span>
    <span class="min-w-0 truncate text-[12px] font-medium text-[#111827] dark:text-[#ececec]">{{ planTitle }}</span>
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="shrink-0 text-[#94a3b8]">
      <path d="M9 6l6 6-6 6" />
    </svg>
  </button>
</template>
