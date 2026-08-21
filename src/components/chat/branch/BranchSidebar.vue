<script setup lang="ts">
// 分支对话侧边栏：挂在主聊天右侧的临时问答面板。
// 视觉对齐主 app 的白底黑字语言（白色卡片 + 浅灰细边框 + 黑色主按钮），
// 复用 shadcn-vue 的 Button / Textarea 组件保持观感一致。
// 数据全部来自 branch-chat.ts 的内存态单例——关闭即丢弃，不落库；
// 唯一的沉淀出口是 header 的「存为会话」按钮。
import { computed, nextTick, ref, watch } from 'vue';
import MarkdownRenderer from '../MarkdownRenderer.vue';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import {
  activeBranch,
  branchesFor,
  cancelBranch,
  closeBranch,
  closeSidebar,
  exportBranchAsConversation,
  sendBranchMessage,
  setActiveBranch,
  sidebarOpen,
  type BranchSession,
} from '../../../features/branch/branch-chat';

const props = defineProps<{
  conversationId?: string | null;
}>();

const branchList = computed(() => branchesFor(props.conversationId));

/** 当前展示的分支：优先激活项；激活项不属于本会话时回落到本会话最新分支。 */
const session = computed<BranchSession | null>(() => {
  const active = activeBranch.value;
  if (active && active.parentConversationId === props.conversationId) {
    return active;
  }
  const list = branchList.value;
  return list.length > 0 ? list[list.length - 1] : null;
});

const visible = computed(() => sidebarOpen.value && !!session.value);

const draft = ref('');
const isComposing = ref(false);
const messagesEl = ref<HTMLElement | null>(null);
// Textarea 是 shadcn-vue 组件，模板 ref 拿到的是组件实例，需经 $el 取真实 DOM。
const composerRef = ref<InstanceType<typeof Textarea> | null>(null);

const composerEl = (): HTMLTextAreaElement | null => {
  const instance = composerRef.value;
  if (!instance) return null;
  return instance.$el instanceof HTMLTextAreaElement ? instance.$el : null;
};

const canSend = computed(() => draft.value.trim().length > 0);

const quotePreview = (text: string, max = 14) => {
  const normalized = text.replace(/\s+/g, ' ').trim();
  return normalized.length > max ? `${normalized.slice(0, max)}…` : normalized;
};

const scrollToBottom = async () => {
  await nextTick();
  const el = messagesEl.value;
  if (el) {
    el.scrollTop = el.scrollHeight;
  }
};

watch(
  () => [session.value?.messages.length ?? 0, session.value?.streamingText.length ?? 0] as const,
  () => {
    void scrollToBottom();
  },
);

watch(visible, async (value) => {
  if (value) {
    await scrollToBottom();
    composerEl()?.focus();
  }
});

const autoResize = () => {
  const el = composerEl();
  if (!el) return;
  el.style.height = 'auto';
  el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
};

const send = () => {
  const current = session.value;
  const text = draft.value.trim();
  if (!current || !text || current.phase === 'running') return;
  draft.value = '';
  void nextTick(autoResize);
  void sendBranchMessage(current, text);
};

const onComposerKeydown = (event: KeyboardEvent) => {
  if (event.key === 'Enter' && !event.shiftKey && !isComposing.value) {
    event.preventDefault();
    send();
  }
};

const handleExport = () => {
  const current = session.value;
  if (current) {
    void exportBranchAsConversation(current);
  }
};

const handleClose = () => {
  const current = session.value;
  if (current) {
    closeBranch(current.branchId);
  } else {
    closeSidebar();
  }
};

// 与子代理抽屉一致：顶部避开全局 header（56px），右侧/底部留边。
const drawerTop = '56px';
</script>

<template>
  <Teleport to="body">
    <Transition name="branch-drawer">
      <aside
        v-if="visible && session"
        class="branch-drawer fixed right-4 bottom-4 z-[45] flex w-[400px] max-w-[calc(100vw-32px)] flex-col overflow-hidden rounded-2xl border border-[#e5e7eb] bg-white/95 shadow-[0_12px_40px_rgba(15,23,42,0.12)] backdrop-blur-[10px] dark:border-[#333] dark:bg-[#242424]/95 dark:shadow-[0_12px_40px_rgba(0,0,0,0.4)]"
        :style="{ top: drawerTop, height: 'calc(100vh - 56px - 32px)' }"
        role="complementary"
        aria-label="分支对话"
      >
        <!-- 头部 -->
        <header class="flex shrink-0 items-center gap-2 border-b border-[#e5e7eb] px-4 py-3 dark:border-[#333]">
          <div class="text-[15px] font-semibold text-[#111827] dark:text-[#ececec]">分支问答</div>
          <div class="flex-1 text-[12px] text-[#94a3b8] dark:text-[#737373]" title="分支内容仅保存在内存，关闭后不保留">
            临时 · 不保存
          </div>
          <Button
            variant="ghost"
            size="sm"
            class="h-7 gap-1 rounded-md px-2 text-[12px] text-[#64748b] hover:bg-[#f1f5f9] hover:text-[#111827] dark:text-[#a3a3a3] dark:hover:bg-white/8 dark:hover:text-[#ececec]"
            :disabled="session.messages.length === 0 || session.phase === 'running' || session.exported"
            :title="session.exported ? '已保存为新会话' : '把分支内容保存为正式会话'"
            @click="handleExport"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
              <polyline points="17 21 17 13 7 13 7 21" />
              <polyline points="7 3 7 8 15 8" />
            </svg>
            <span>{{ session.exported ? '已保存' : '存为会话' }}</span>
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            class="h-7 w-7 rounded-md text-[#64748b] hover:bg-[#f1f5f9] hover:text-[#111827] dark:text-[#a3a3a3] dark:hover:bg-white/8 dark:hover:text-[#ececec]"
            aria-label="关闭分支面板"
            title="关闭并丢弃该分支"
            @click="handleClose"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </Button>
        </header>

        <!-- 多分支切换 chips -->
        <div v-if="branchList.length > 1" class="flex shrink-0 gap-1.5 overflow-x-auto px-4 pt-2.5">
          <button
            v-for="item in branchList"
            :key="item.branchId"
            type="button"
            class="inline-flex shrink-0 cursor-pointer items-center gap-1 whitespace-nowrap rounded-full border px-2.5 py-1 text-[11px] transition-colors"
            :class="item.branchId === session.branchId
              ? 'border-[#cbd5e1] bg-[#f1f5f9] font-semibold text-[#111827] dark:border-[#4a4a4a] dark:bg-white/8 dark:text-[#ececec]'
              : 'border-[#e2e8f0] bg-[#f8fafc] text-[#64748b] hover:border-[#cbd5e1] dark:border-[#333] dark:bg-white/4 dark:text-[#94a3b8] dark:hover:border-[#4a4a4a]'"
            :title="item.quotedText"
            @click="setActiveBranch(item.branchId)"
          >
            <span>{{ quotePreview(item.quotedText) }}</span>
            <span
              class="inline-flex h-3.5 w-3.5 items-center justify-center rounded-full opacity-55 transition-opacity hover:opacity-100 hover:bg-black/8 dark:hover:bg-white/10"
              role="button"
              aria-label="关闭分支"
              @click.stop="closeBranch(item.branchId)"
            >
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" aria-hidden="true">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </span>
          </button>
        </div>

        <!-- 引用内容 -->
        <div
          class="mx-4 mt-2.5 shrink-0 rounded-xl border border-[#e5e7eb] border-l-[3px] border-l-[#cbd5e1] bg-[#f8fafc] px-3 py-2.5 dark:border-[#333] dark:border-l-[#4a4a4a] dark:bg-white/4"
          :title="session.quotedText"
        >
          <div class="mb-1 text-[10px] uppercase tracking-[0.04em] text-[#94a3b8] dark:text-[#737373]">引用自主对话</div>
          <div class="max-h-[84px] overflow-y-auto whitespace-pre-wrap break-words text-[12px] leading-relaxed text-[#475569] dark:text-[#d4d4d4]">
            {{ session.quotedText }}
          </div>
        </div>

        <!-- 消息区 -->
        <div ref="messagesEl" class="branch-scrollbar flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 py-3.5">
          <template v-for="(message, index) in session.messages" :key="index">
            <div v-if="message.role === 'user'" class="flex justify-end">
              <div class="max-w-[88%] whitespace-pre-wrap break-words rounded-xl border border-[#e5e7eb] bg-[#f3f4f6] px-3 py-2 text-[0.92rem] leading-relaxed text-[#111827] dark:border-[#3c3c3c] dark:bg-[#2d2d2d] dark:text-[#ececec]">
                {{ message.content }}
              </div>
            </div>
            <div v-else class="min-w-0 text-[13px] leading-[1.7] text-[#1a1a1a] dark:text-[#ececec]">
              <MarkdownRenderer :content="message.content" />
            </div>
          </template>

          <div v-if="session.phase === 'running'" class="min-w-0 text-[13px] leading-[1.7] text-[#1a1a1a] dark:text-[#ececec]">
            <p v-if="!session.streamingText" class="m-0 inline-flex items-center gap-1.5 text-[12px] text-[#94a3b8]">
              正在思考<span class="branch-thinking__dots" aria-hidden="true"><span></span><span></span><span></span></span>
            </p>
            <template v-else>
              <MarkdownRenderer :content="session.streamingText" live />
              <span class="branch-cursor"></span>
            </template>
          </div>

          <div
            v-if="session.error"
            class="whitespace-pre-wrap break-words rounded-xl border border-[#fecaca] bg-[#fef2f2] px-3 py-2 text-[12.5px] leading-[1.55] text-[#b91c1c] dark:border-[rgba(239,68,68,0.28)] dark:bg-[rgba(239,68,68,0.1)] dark:text-[#fca5a5]"
          >
            {{ session.error }}
          </div>

          <p v-if="session.messages.length === 0 && session.phase !== 'running'" class="m-auto px-3 py-6 text-center text-[13px] text-[#94a3b8] dark:text-[#737373]">
            针对上方引用内容提问，不会污染主对话上下文。
          </p>
        </div>

        <!-- 输入区 -->
        <footer class="shrink-0 border-t border-[#e5e7eb] px-4 pb-3.5 pt-3 dark:border-[#333]">
          <div class="flex items-end gap-2 rounded-2xl border border-[#e5e7eb] bg-white p-2 pl-3 shadow-[0_1px_2px_rgba(15,23,42,0.04)] transition-colors focus-within:border-[#cbd5e1] dark:border-[#3a3a3a] dark:bg-[#2a2a2a] dark:shadow-none dark:focus-within:border-[#4a4a4a]">
            <Textarea
              ref="composerRef"
              v-model="draft"
              rows="1"
              placeholder="就引用内容继续追问…"
              class="max-h-[120px] min-h-0 flex-1 resize-none border-0 bg-transparent p-1 text-[13px] leading-[1.5] shadow-none focus-visible:ring-0 dark:bg-transparent"
              @input="autoResize"
              @keydown="onComposerKeydown"
              @compositionstart="isComposing = true"
              @compositionend="isComposing = false"
            />
            <Button
              size="icon-sm"
              :variant="session.phase === 'running' ? 'destructive' : 'default'"
              class="h-8 w-8 shrink-0 rounded-full"
              :class="session.phase === 'running'
                ? 'bg-[#fee2e2] text-[#b91c1c] hover:bg-[#fecaca]'
                : canSend
                  ? 'bg-[#111827] text-white hover:bg-[#1f2937] dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white'
                  : 'bg-[#f1f5f9] text-[#94a3b8] dark:bg-[#333] dark:text-[#737373]'"
              :disabled="session.phase !== 'running' && !canSend"
              :title="session.phase === 'running' ? '停止生成' : '发送'"
              @click="session.phase === 'running' ? void cancelBranch(session) : send()"
            >
              <svg
                v-if="session.phase === 'running'"
                width="12" height="12" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"
              >
                <rect x="6" y="6" width="12" height="12" rx="2" />
              </svg>
              <svg
                v-else
                width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
              >
                <line x1="12" y1="19" x2="12" y2="5" />
                <polyline points="5 12 12 5 19 12" />
              </svg>
            </Button>
          </div>
        </footer>
      </aside>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* 抽屉进出场动画 */
.branch-drawer-enter-active,
.branch-drawer-leave-active {
  transition: opacity 180ms ease, transform 180ms ease;
}

.branch-drawer-enter-from,
.branch-drawer-leave-to {
  opacity: 0;
  transform: translateX(24px);
}

/* 细滚动条 */
.branch-scrollbar::-webkit-scrollbar {
  width: 6px;
}

.branch-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}

.branch-scrollbar::-webkit-scrollbar-thumb {
  background: #e2e8f0;
  border-radius: 999px;
}

.dark .branch-scrollbar::-webkit-scrollbar-thumb {
  background: #4b5563;
}

/* 思考中跳动圆点 */
.branch-thinking__dots {
  display: inline-flex;
  align-items: flex-end;
  gap: 4px;
}

.branch-thinking__dots span {
  width: 4px;
  height: 4px;
  border-radius: 999px;
  background: currentColor;
  opacity: 0.45;
  animation: branch-dot-bounce 1s ease-in-out infinite;
}

.branch-thinking__dots span:nth-child(2) {
  animation-delay: 0.15s;
}

.branch-thinking__dots span:nth-child(3) {
  animation-delay: 0.3s;
}

@keyframes branch-dot-bounce {
  0%, 80%, 100% {
    transform: translateY(0);
    opacity: 0.35;
  }
  40% {
    transform: translateY(-3px);
    opacity: 0.9;
  }
}

/* 流式光标 */
.branch-cursor {
  display: inline-block;
  width: 6px;
  height: 1em;
  margin-left: 2px;
  vertical-align: middle;
  background: currentColor;
  opacity: 0.6;
  animation: branch-cursor-blink 0.9s steps(2) infinite;
}

@keyframes branch-cursor-blink {
  50% {
    opacity: 0;
  }
}
</style>
