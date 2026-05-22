<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { UnlistenFn } from '@tauri-apps/api/event';
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
const toolbarStatus = computed(() => (isBrowserWindowReady.value ? '独立窗口顶部' : '打开后显示'));

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
  <div class="flex h-full min-h-0 flex-col bg-white p-5 text-[#1f2328] dark:bg-[#171717] dark:text-[#ececec]">
    <div class="flex min-h-0 flex-1 items-center justify-center">
      <div class="w-full max-w-[560px] rounded-[24px] border border-[#e7e2d7] bg-[#fffdf8] p-6 shadow-sm dark:border-[#333] dark:bg-[#222]">
        <div class="flex items-center gap-3">
          <div class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-[#edf6ff] text-[#0a84ff] dark:bg-[#10253a]">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="15" rx="2" />
            <path d="M3 9h18" />
            <path d="M8 21h8" />
          </svg>
          </div>
          <div class="text-left">
            <h3 class="text-[17px] font-semibold text-[#24282d] dark:text-[#f2f2f2]">
              当前浏览器状态
            </h3>
            <div class="mt-1 text-xs text-[#92887a] dark:text-[#9f978c]">
              Nova Browser
            </div>
          </div>
        </div>

        <div class="mt-5 grid gap-2 rounded-2xl border border-[#eee7dc] bg-white p-3 text-left text-xs dark:border-[#3a3a3a] dark:bg-[#1b1b1b]">
          <div class="flex items-center justify-between gap-3">
            <span class="text-[#92887a] dark:text-[#9f978c]">窗口状态</span>
            <span
              class="browser-window-status"
              :class="isBrowserWindowReady ? 'browser-window-status-open' : 'browser-window-status-closed'"
            >
              {{ isBrowserWindowReady ? '已打开' : '未打开' }}
            </span>
          </div>
          <div class="flex items-center justify-between gap-3">
            <span class="text-[#92887a] dark:text-[#9f978c]">会话</span>
            <span class="min-w-0 truncate text-right text-[#5d554b] dark:text-[#d0c7ba]" :title="conversationLabel">
              {{ conversationLabel }}
            </span>
          </div>
          <div class="flex items-start justify-between gap-3">
            <span class="shrink-0 text-[#92887a] dark:text-[#9f978c]">当前页面</span>
            <span
              v-if="displayUrl"
              class="min-w-0 truncate text-right text-[#5d554b] dark:text-[#d0c7ba]"
              :title="displayUrl"
            >
              {{ displayUrl }}
            </span>
            <span v-else class="text-right text-[#b0a79a] dark:text-[#8f877c]">
              打开窗口后输入 URL
            </span>
          </div>
          <div class="flex items-center justify-between gap-3">
            <span class="text-[#92887a] dark:text-[#9f978c]">缩放</span>
            <span class="text-right text-[#5d554b] dark:text-[#d0c7ba]">{{ zoomPercent }}%</span>
          </div>
          <div class="flex items-center justify-between gap-3">
            <span class="text-[#92887a] dark:text-[#9f978c]">工具栏</span>
            <span class="text-right text-[#5d554b] dark:text-[#d0c7ba]">{{ toolbarStatus }}</span>
          </div>
          <div class="flex items-center justify-between gap-3">
            <span class="text-[#92887a] dark:text-[#9f978c]">Agent 自动化</span>
            <span class="text-right text-[#5d554b] dark:text-[#d0c7ba]">{{ automationStatus }}</span>
          </div>
        </div>

        <div class="mt-5 flex flex-wrap justify-center gap-2">
          <button type="button" class="browser-primary-action" @click="openOrFocusBrowserWindow">
            {{ isBrowserWindowReady ? '聚焦浏览器窗口' : '打开浏览器窗口' }}
          </button>
          <button
            type="button"
            class="browser-secondary-action"
            :disabled="!isBrowserWindowReady"
            @click="closeBrowserWindowFromUi"
          >
            关闭窗口
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.browser-window-status {
  display: inline-flex;
  align-items: center;
  border-radius: 9999px;
  padding: 0.18rem 0.55rem;
  font-weight: 700;
}

.browser-window-status-open {
  background: #e7f7ed;
  color: #177245;
}

.browser-window-status-closed {
  background: #f2eee7;
  color: #8a7865;
}

.browser-primary-action,
.browser-secondary-action {
  display: inline-flex;
  min-height: 2.25rem;
  align-items: center;
  justify-content: center;
  border-radius: 0.85rem;
  padding: 0 0.9rem;
  font-size: 0.85rem;
  font-weight: 600;
  transition: transform 0.15s ease, background-color 0.15s ease, color 0.15s ease, opacity 0.15s ease;
}

.browser-primary-action {
  background: #0a84ff;
  color: white;
}

.browser-primary-action:hover:not(:disabled) {
  background: #0877e6;
  transform: translateY(-1px);
}

.browser-secondary-action {
  border: 1px solid #ded8cf;
  background: #fff;
  color: #5d554b;
}

.browser-secondary-action:hover:not(:disabled) {
  background: #f2eee6;
  transform: translateY(-1px);
}

.browser-primary-action:disabled,
.browser-secondary-action:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}
</style>
