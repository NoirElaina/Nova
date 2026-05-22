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
  <div class="flex h-full min-h-0 flex-col bg-white text-[#111827] dark:bg-[#171717] dark:text-[#ececec]">
    <div class="flex h-12 shrink-0 items-center justify-between border-b border-[#e5e7eb] px-4 dark:border-[#333]">
      <div class="flex min-w-0 items-center gap-2.5">
        <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[#f3f6fa] text-[#64748b] dark:bg-[#262626] dark:text-[#b8b8b8]">
          <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="15" rx="2" />
            <path d="M3 9h18" />
            <path d="M8 21h8" />
          </svg>
        </span>
        <div class="min-w-0">
          <div class="truncate text-[13px] font-medium">Nova Browser</div>
          <div class="truncate text-[11px] text-[#64748b] dark:text-[#aaa]">
            Agent 自动化和页面注释入口
          </div>
        </div>
      </div>

      <div
        class="inline-flex h-7 items-center rounded-full px-2.5 text-[12px] font-medium"
        :class="isBrowserWindowReady
          ? 'bg-[#ecfdf5] text-[#047857] dark:bg-[#123225] dark:text-[#86efac]'
          : 'bg-[#f3f4f6] text-[#64748b] dark:bg-[#2d2d2d] dark:text-[#bdbdbd]'"
      >
        {{ isBrowserWindowReady ? '窗口已打开' : '未打开' }}
      </div>
    </div>

    <div class="min-h-0 flex-1 overflow-auto p-4">
      <div class="grid gap-3">
        <div class="rounded-xl border border-[#e5e7eb] bg-white p-3 shadow-[0_1px_2px_rgba(15,23,42,0.035)] dark:border-[#333] dark:bg-[#222]">
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="text-[13px] font-medium text-[#111827] dark:text-[#ececec]">当前页面</div>
              <div
                class="mt-1 truncate text-[12px] text-[#64748b] dark:text-[#aaa]"
                :title="displayUrl || undefined"
              >
                {{ displayUrl || '尚未打开页面' }}
              </div>
            </div>
            <span class="shrink-0 rounded-md bg-[#f8fafc] px-2 py-1 text-[11px] text-[#64748b] ring-1 ring-[#e5e7eb] dark:bg-[#262626] dark:ring-[#333]">
              {{ zoomPercent }}%
            </span>
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="rounded-xl border border-[#e5e7eb] bg-[#fbfcfe] p-3 dark:border-[#333] dark:bg-[#202020]">
            <div class="text-[11px] text-[#64748b] dark:text-[#aaa]">会话</div>
            <div class="mt-1 truncate text-[13px] font-medium text-[#111827] dark:text-[#ececec]" :title="conversationLabel">
              {{ conversationLabel }}
            </div>
          </div>
          <div class="rounded-xl border border-[#e5e7eb] bg-[#fbfcfe] p-3 dark:border-[#333] dark:bg-[#202020]">
            <div class="text-[11px] text-[#64748b] dark:text-[#aaa]">Agent 自动化</div>
            <div class="mt-1 text-[13px] font-medium text-[#111827] dark:text-[#ececec]">
              {{ automationStatus }}
            </div>
          </div>
          <div class="rounded-xl border border-[#e5e7eb] bg-[#fbfcfe] p-3 dark:border-[#333] dark:bg-[#202020]">
            <div class="text-[11px] text-[#64748b] dark:text-[#aaa]">工具栏</div>
            <div class="mt-1 text-[13px] font-medium text-[#111827] dark:text-[#ececec]">
              {{ toolbarStatus }}
            </div>
          </div>
          <div class="rounded-xl border border-[#e5e7eb] bg-[#fbfcfe] p-3 dark:border-[#333] dark:bg-[#202020]">
            <div class="text-[11px] text-[#64748b] dark:text-[#aaa]">页面注释</div>
            <div class="mt-1 text-[13px] font-medium text-[#111827] dark:text-[#ececec]">
              独立窗口中选择元素
            </div>
          </div>
        </div>

        <div class="flex flex-wrap gap-2 pt-1">
          <Button
            type="button"
            size="sm"
            class="h-8 rounded-lg bg-[#111827] px-3 text-[13px] font-medium text-white hover:bg-[#1f2937]"
            @click="openOrFocusBrowserWindow"
          >
            {{ isBrowserWindowReady ? '聚焦浏览器窗口' : '打开浏览器窗口' }}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            class="h-8 rounded-lg border-[#e5e7eb] bg-white px-3 text-[13px] font-medium text-[#475569] hover:bg-[#f8fafc] dark:border-[#333] dark:bg-[#222] dark:text-[#d7d7d7] dark:hover:bg-[#2a2a2a]"
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
