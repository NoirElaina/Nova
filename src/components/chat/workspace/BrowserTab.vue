<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { Button } from '@/components/ui/button';
import {
  browserStateKey,
  clearBrowserTabState,
  getBrowserTabState,
  listenBrowserTabStateCleared,
  loadBrowserTabState,
  type BrowserTabState,
} from '../../../features/browser/browser-tab-state';
import { useNativeBrowserWindow } from '../../../features/browser/useNativeBrowserWindow';

const props = defineProps<{
  conversationId?: string | null;
  visible?: boolean;
  openRequestKey?: number;
}>();

const initialState = getBrowserTabState(props.conversationId);
const currentUrl = ref(initialState?.currentUrl ?? '');
const addressInput = ref(initialState?.addressInput ?? '');
const isLoading = ref(false);
const zoomPercent = ref(initialState?.zoomPercent ?? 100);
const browserLabel = `nova-browser-page-${crypto.randomUUID()}`;
let unlistenBrowserStateCleared: UnlistenFn | null = null;

const displayUrl = computed(() => currentUrl.value || addressInput.value || '');
const conversationLabel = computed(() => props.conversationId?.trim() || '默认会话');
const automationStatus = computed(() => (isBrowserWindowReady.value ? '可用' : '等待窗口'));
const toolbarStatus = computed(() => (isBrowserWindowReady.value ? '已在独立窗口显示' : '打开窗口后显示'));

const applyBrowserState = (state: BrowserTabState | null) => {
  currentUrl.value = state?.currentUrl ?? '';
  addressInput.value = state?.addressInput ?? '';
  zoomPercent.value = state?.zoomPercent ?? 100;
};

const restoreBrowserState = async () => {
  applyBrowserState(await loadBrowserTabState(props.conversationId));
};

const {
  isBrowserWindowReady,
  closeBrowserWindow,
  focusBrowserWindow,
} = useNativeBrowserWindow({
  browserLabel,
  conversationId: () => props.conversationId || null,
  currentUrl,
  isLoading,
  shouldAutoRestore: () => props.visible !== false,
});

const openOrFocusBrowserWindow = async () => {
  await restoreBrowserState();
  await focusBrowserWindow();
};

const closeBrowserWindowFromUi = async () => {
  await closeBrowserWindow();
  applyBrowserState(null);
  await clearBrowserTabState(props.conversationId);
};

watch(
  () => props.visible,
  (visible) => {
    if (visible === false) return;
    void restoreBrowserState();
  },
);

watch(
  () => props.openRequestKey,
  (next, previous) => {
    if (!next || next === previous) return;
    void openOrFocusBrowserWindow();
  },
);

watch(
  () => props.conversationId,
  () => {
    void restoreBrowserState();
  },
);

onMounted(() => {
  void restoreBrowserState();
  void listenBrowserTabStateCleared((payload) => {
    if (payload.key === browserStateKey(props.conversationId)) {
      applyBrowserState(null);
    }
  }).then((unlisten) => {
    unlistenBrowserStateCleared = unlisten;
  }).catch((error) => {
    console.warn('Browser tab state clear listener failed:', error);
  });
});

onBeforeUnmount(() => {
  unlistenBrowserStateCleared?.();
});

</script>

<template>
  <div class="flex h-full min-h-0 flex-col text-[#111827] dark:text-[#ececec]">
    <!-- 顶栏：标题 + 状态胶囊 -->
    <div class="flex h-10 shrink-0 items-center justify-between border-b border-[#eef0f3] px-3 dark:border-[#2c2c2c]">
      <div class="min-w-0 text-[13px] font-medium">Nova 浏览器</div>
      <span
        class="inline-flex h-6 shrink-0 items-center rounded-full px-2.5 text-[11px] font-medium"
        :class="isBrowserWindowReady
          ? 'bg-[#ecfdf5] text-[#047857] dark:bg-[#123225] dark:text-[#86efac]'
          : 'bg-[#f3f4f6] text-[#64748b] dark:bg-white/5 dark:text-[#bdbdbd]'"
      >
        {{ isBrowserWindowReady ? '窗口已打开' : '未打开' }}
      </span>
    </div>

    <div class="min-h-0 flex-1 overflow-auto p-3">
      <div class="flex flex-col gap-3">
        <!-- 当前页面 -->
        <div class="rounded-xl border border-[#e7e9ee] bg-[#fafbfc] px-3 py-2.5 dark:border-[#343434] dark:bg-white/5">
          <div class="flex items-center justify-between gap-3">
            <span class="shrink-0 text-[11px] text-[#64748b] dark:text-[#aaa]">当前页面</span>
            <span class="shrink-0 rounded bg-white px-1.5 py-0.5 text-[11px] text-[#64748b] ring-1 ring-[#e7e9ee] dark:bg-[#262626] dark:ring-[#333]">
              {{ zoomPercent }}%
            </span>
          </div>
          <div
            class="mt-1 break-all font-mono text-[12px] leading-relaxed text-[#334155] dark:text-[#ccc]"
            :title="displayUrl || undefined"
          >
            {{ displayUrl || '尚未打开页面' }}
          </div>
        </div>

        <!-- 状态信息：单卡多行，左标签右值 -->
        <div class="rounded-xl border border-[#e7e9ee] dark:border-[#343434]">
          <div class="flex items-center justify-between gap-3 px-3 py-2">
            <span class="text-[12px] text-[#64748b] dark:text-[#aaa]">会话</span>
            <span class="min-w-0 truncate text-[12px] font-medium" :title="conversationLabel">{{ conversationLabel }}</span>
          </div>
          <div class="flex items-center justify-between gap-3 border-t border-[#eef0f3] px-3 py-2 dark:border-[#2c2c2c]">
            <span class="text-[12px] text-[#64748b] dark:text-[#aaa]">Agent 自动化</span>
            <span class="text-[12px] font-medium">{{ automationStatus }}</span>
          </div>
          <div class="flex items-center justify-between gap-3 border-t border-[#eef0f3] px-3 py-2 dark:border-[#2c2c2c]">
            <span class="text-[12px] text-[#64748b] dark:text-[#aaa]">工具栏</span>
            <span class="text-[12px] font-medium">{{ toolbarStatus }}</span>
          </div>
          <div class="flex items-center justify-between gap-3 border-t border-[#eef0f3] px-3 py-2 dark:border-[#2c2c2c]">
            <span class="text-[12px] text-[#64748b] dark:text-[#aaa]">页面注释</span>
            <span class="text-[12px] font-medium">独立窗口中选择元素</span>
          </div>
        </div>

        <!-- 操作按钮 -->
        <div class="flex gap-2">
          <Button
            type="button"
            size="sm"
            class="h-8 flex-1 rounded-lg bg-[#111827] text-[13px] font-medium text-white hover:bg-[#1f2937]"
            @click="openOrFocusBrowserWindow"
          >
            {{ isBrowserWindowReady ? '聚焦浏览器窗口' : '打开浏览器窗口' }}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            class="h-8 rounded-lg border-[#e7e9ee] px-3 text-[13px] font-medium text-[#475569] hover:bg-[#f8fafc] dark:border-[#343434] dark:text-[#d7d7d7] dark:hover:bg-white/5"
            :disabled="!isBrowserWindowReady"
            @click="closeBrowserWindowFromUi"
          >
            关闭窗口
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>
