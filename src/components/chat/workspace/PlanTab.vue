<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import MarkdownRenderer from '../MarkdownRenderer.vue';
import { getConversationPlan } from '../../../features/chat/services/chat-api';

const props = defineProps<{
  conversationId: string | null;
}>();

/** plan 文件由 exit_plan_mode 工具写入；这里只负责读取与实时刷新。 */
const planContent = ref('');
const planUpdatedAt = ref(0);

let unlistenPlanUpdated: UnlistenFn | null = null;

const loadPlan = async () => {
  const target = props.conversationId;
  try {
    const plan = await getConversationPlan(target);
    // 会话可能已经切走，避免旧请求覆盖新会话的 plan。
    if (target !== props.conversationId) return;
    planContent.value = plan?.content ?? '';
    planUpdatedAt.value = plan?.updatedAt ?? 0;
  } catch {
    if (target === props.conversationId) {
      planContent.value = '';
      planUpdatedAt.value = 0;
    }
  }
};

watch(() => props.conversationId, () => {
  planContent.value = '';
  planUpdatedAt.value = 0;
  void loadPlan();
}, { immediate: true });

onMounted(async () => {
  unlistenPlanUpdated = await listen<{
    conversationId?: string | null;
    content?: string;
    updatedAt?: number;
  }>('plan-updated', (event) => {
    const payload = event.payload;
    const current = props.conversationId;
    // 无会话上下文（__default__）或匹配当前会话时才刷新。
    if (current && payload.conversationId && payload.conversationId !== current) {
      return;
    }
    if (typeof payload.content === 'string') {
      planContent.value = payload.content;
      planUpdatedAt.value = payload.updatedAt ?? Date.now() / 1000;
    } else {
      void loadPlan();
    }
  });
});

onBeforeUnmount(() => {
  unlistenPlanUpdated?.();
});

const formatUpdatedAt = (seconds: number) => {
  if (!seconds || seconds <= 0) return '';
  const date = new Date(seconds * 1000);
  if (Number.isNaN(date.getTime())) return '';
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
};

type PlanSection = {
  heading: string;
  body: string;
};

/**
 * 把 plan markdown 拆成「标题 + 引言 + 分节」：
 * `# ` 一行作为面板标题，`## ` 作为可折叠分节（Context/目标/步骤/验证等），
 * 与参考设计的结构化计划面板一致。
 */
const parsedPlan = computed(() => {
  const text = planContent.value.trim();
  if (!text) {
    return { title: '', intro: '', sections: [] as PlanSection[] };
  }
  const lines = text.split(/\r?\n/);
  let title = '';
  let introStart = 0;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (!line.trim()) continue;
    if (/^#\s+/.test(line)) {
      title = line.replace(/^#\s+/, '').trim();
      introStart = i + 1;
    } else {
      introStart = i;
    }
    break;
  }

  const introLines: string[] = [];
  const sections: PlanSection[] = [];
  let current: { heading: string; lines: string[] } | null = null;
  for (let i = introStart; i < lines.length; i += 1) {
    const line = lines[i];
    const headingMatch = /^##\s+(.+)$/.exec(line);
    if (headingMatch) {
      if (current) {
        sections.push({ heading: current.heading, body: current.lines.join('\n').trim() });
      }
      current = { heading: headingMatch[1].trim(), lines: [] };
      continue;
    }
    if (current) {
      current.lines.push(line);
    } else {
      introLines.push(line);
    }
  }
  if (current) {
    sections.push({ heading: current.heading, body: current.lines.join('\n').trim() });
  }

  return {
    title,
    intro: introLines.join('\n').trim(),
    sections,
  };
});

const hasPlan = computed(() => planContent.value.trim().length > 0);
</script>

<template>
  <div class="h-full overflow-y-auto px-6 py-5 custom-scrollbar">
    <!-- 无计划时的空状态 -->
    <div
      v-if="!hasPlan"
      class="flex h-full flex-col items-center justify-center gap-2 text-center"
    >
      <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" class="text-[#94a3b8]">
        <rect x="8" y="2" width="8" height="4" rx="1"/>
        <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/>
        <path d="M9 12h6M9 16h4"/>
      </svg>
      <p class="text-[13px] text-[#64748b] dark:text-[#9ca3af]">该会话还没有计划</p>
      <p class="text-[12px] text-[#94a3b8] dark:text-[#737373]">在 Plan 模式下让 AI 制定计划后，会自动显示在这里</p>
    </div>

    <!-- 结构化计划：纯排版展示（大标题 + 分节加粗标题），
         与参考应用一致：无外框卡片、无标题栏、无折叠箭头。 -->
    <div v-else class="mx-auto w-full max-w-[760px]">
      <!-- 计划大标题 -->
      <div class="mb-4 flex items-baseline gap-3">
        <h1 class="min-w-0 flex-1 truncate text-[17px] font-semibold leading-snug text-[#111827] dark:text-[#f0f0f0]">
          {{ parsedPlan.title || '计划' }}
        </h1>
        <span
          v-if="planUpdatedAt > 0"
          class="shrink-0 text-[11px] text-[#94a3b8] dark:text-[#737373]"
        >{{ formatUpdatedAt(planUpdatedAt) }}</span>
      </div>

      <!-- 引言（第一个 ## 之前的内容） -->
      <MarkdownRenderer
        v-if="parsedPlan.intro"
        :content="parsedPlan.intro"
        class="plan-intro mb-4"
      />

      <!-- 无分节时直接整体渲染 -->
      <MarkdownRenderer
        v-if="parsedPlan.sections.length === 0 && !parsedPlan.intro"
        :content="planContent"
      />

      <!-- 分节：加粗标题 + 内容直接展示（目标 / 实施步骤 / 技术决策 / 验证方式 等） -->
      <section
        v-for="(section, index) in parsedPlan.sections"
        :key="`${section.heading}-${index}`"
        class="plan-section mb-4"
      >
        <h2 class="mb-1.5 text-[14px] font-semibold text-[#111827] dark:text-[#ececec]">{{ section.heading }}</h2>
        <MarkdownRenderer :content="section.body" />
      </section>
    </div>
  </div>
</template>

<style scoped>
.custom-scrollbar::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}
.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: var(--color-border, #e5e5e5);
  border-radius: 10px;
}
.dark .custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: #444;
}

/* 分节内的 markdown 列表/段落收紧，贴合轻量风格 */
.plan-section :deep(p),
.plan-intro :deep(p) {
  margin: 0.3rem 0;
}
.plan-section :deep(ul),
.plan-section :deep(ol) {
  margin: 0.3rem 0;
  padding-left: 1.4em;
}
.plan-section :deep(li) {
  margin: 0.15rem 0;
}
</style>
