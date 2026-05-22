<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  getBrowserTabState,
  loadBrowserTabState,
  saveBrowserTabState,
  type BrowserTabState,
} from '../../../features/browser/browser-tab-state';
import { useBrowserAutomationCommands } from '../../../features/browser/useBrowserAutomationCommands';
import { useBrowserPageWebview } from '../../../features/browser/useBrowserPageWebview';
import { useBrowserSnapshot } from '../../../features/browser/useBrowserSnapshot';

const params = new URLSearchParams(window.location.search);
const pageLabel = params.get('pageLabel') || `nova-browser-page-${crypto.randomUUID()}`;
const conversationId = params.get('conversationId') || null;
const initialUrl = params.get('initialUrl') || '';
const currentWindow = getCurrentWindow();

const initialState = getBrowserTabState(conversationId);
const addressInput = ref(initialUrl || initialState?.addressInput || '');
const currentUrl = ref(initialUrl || initialState?.currentUrl || '');
const history = ref<string[]>(initialState?.history ?? []);
const historyIndex = ref(initialState?.historyIndex ?? -1);
const isLoading = ref(false);
const isMenuOpen = ref(false);
const zoomPercent = ref(initialState?.zoomPercent ?? 100);
const isElementPickerActive = ref(false);
const browserHost = ref<HTMLElement | null>(null);
let resizeObserver: ResizeObserver | null = null;
let pickerRetryTimers: number[] = [];
let unlistenBrowserCommand: UnlistenFn | null = null;

const canGoBack = computed(() => historyIndex.value > 0);
const canGoForward = computed(() => historyIndex.value >= 0 && historyIndex.value < history.value.length - 1);
const browserConversationId = computed(() => conversationId?.trim() || '__default__');

const persistBrowserState = () => {
  saveBrowserTabState(conversationId, {
    addressInput: addressInput.value,
    currentUrl: currentUrl.value,
    history: history.value,
    historyIndex: historyIndex.value,
    zoomPercent: zoomPercent.value,
  });
};

const applyBrowserState = (state: BrowserTabState | null) => {
  addressInput.value = initialUrl || state?.addressInput || '';
  currentUrl.value = initialUrl || state?.currentUrl || '';
  history.value = state?.history ?? [];
  historyIndex.value = state?.historyIndex ?? -1;
  zoomPercent.value = state?.zoomPercent ?? 100;
};

const registerBrowserSession = async () => {
  await invoke('register_browser_session', {
    conversationId,
    label: pageLabel,
    currentUrl: currentUrl.value || null,
  }).catch((error) => {
    console.warn('Browser session registration failed:', error);
  });
};

const unregisterBrowserSession = async () => {
  await invoke('unregister_browser_session', {
    conversationId,
    label: pageLabel,
  }).catch((error) => {
    console.warn('Browser session unregister failed:', error);
  });
};

const updateBrowserSessionUrl = async () => {
  await invoke('update_browser_session_url', {
    conversationId,
    label: pageLabel,
    currentUrl: currentUrl.value || null,
  }).catch((error) => {
    console.warn('Browser session URL update failed:', error);
  });
};

const looksLikeUrl = (value: string) =>
  /^https?:\/\//i.test(value) ||
  /^localhost(:\d+)?(\/.*)?$/i.test(value) ||
  /^\d{1,3}(\.\d{1,3}){3}(:\d+)?(\/.*)?$/.test(value) ||
  /^[^\s]+\.[^\s]+/.test(value);

const normalizeAddress = (raw: string) => {
  const value = raw.trim();
  if (!value) return '';
  if (/^https?:\/\//i.test(value)) return value;
  if (looksLikeUrl(value)) return `https://${value}`;
  return `https://www.google.com/search?q=${encodeURIComponent(value)}`;
};

let schedulePickerInjection = () => {};

const {
  pageWebview,
  isPageWebviewReady,
  navigatePageWebview,
  focusPageWebview,
  closePageWebview,
  evalBrowserScript,
  reloadPageWebview,
  clearBrowsingData,
  applyZoom,
  syncPageWebviewBounds,
} = useBrowserPageWebview({
  pageLabel,
  browserHost,
  currentUrl,
  isLoading,
  zoomPercent,
  onReady: () => schedulePickerInjection(),
  onAfterNavigate: () => schedulePickerInjection(),
});

const {
  callDevtools,
  flattenBrowserFrameTree,
  createFrameExecutionContext,
  captureBrowserSnapshot,
  clickSnapshotRef,
  typeSnapshotRef,
} = useBrowserSnapshot(pageLabel, currentUrl);

const visit = (raw: string, pushHistory = true) => {
  const nextUrl = normalizeAddress(raw);
  if (!nextUrl) return;

  currentUrl.value = nextUrl;
  addressInput.value = nextUrl;
  isLoading.value = true;
  void navigatePageWebview(nextUrl);

  if (!pushHistory) return;
  const nextHistory = history.value.slice(0, historyIndex.value + 1);
  if (nextHistory[nextHistory.length - 1] !== nextUrl) {
    nextHistory.push(nextUrl);
  }
  history.value = nextHistory;
  historyIndex.value = nextHistory.length - 1;
};

const submitAddress = () => visit(addressInput.value);

const goBack = () => {
  if (!canGoBack.value) return;
  historyIndex.value -= 1;
  visit(history.value[historyIndex.value], false);
};

const goForward = () => {
  if (!canGoForward.value) return;
  historyIndex.value += 1;
  visit(history.value[historyIndex.value], false);
};

const reload = () => {
  if (!currentUrl.value) return;
  if (!pageWebview.value || !isPageWebviewReady.value) {
    void navigatePageWebview(currentUrl.value);
    return;
  }
  isLoading.value = true;
  void reloadPageWebview()
    .catch((error) => {
      console.warn('Browser reload failed:', error);
    })
    .finally(() => {
      window.setTimeout(() => {
        isLoading.value = false;
        schedulePickerInjection();
      }, 800);
    });
};

const openExternal = async () => {
  if (!currentUrl.value) return;
  await openUrl(currentUrl.value);
  isMenuOpen.value = false;
};

const forceReload = () => {
  reload();
  isMenuOpen.value = false;
};

const changeZoom = (delta: number) => {
  zoomPercent.value = Math.min(200, Math.max(50, zoomPercent.value + delta));
};

const clearCookies = () => {
  if (isPageWebviewReady.value) {
    void clearBrowsingData().catch((error) => {
      console.warn('Browser clear browsing data failed:', error);
    });
  }
  isMenuOpen.value = false;
};

const clearCache = () => {
  if (isPageWebviewReady.value) {
    void clearBrowsingData().catch((error) => {
      console.warn('Browser clear cache failed:', error);
    });
  }
  isLoading.value = Boolean(currentUrl.value);
  reload();
  isMenuOpen.value = false;
};

const focusPageFromToolbar = () => {
  void focusPageWebview();
  isMenuOpen.value = false;
};

const closeBrowserShell = () => {
  void currentWindow.close();
};

const toggleElementPicker = async () => {
  if (!currentUrl.value && addressInput.value.trim()) {
    submitAddress();
  }
  if (!currentUrl.value) return;
  if (!isPageWebviewReady.value) {
    await focusPageWebview();
  }

  isElementPickerActive.value = !isElementPickerActive.value;
  isMenuOpen.value = false;
  void applyElementPickerState();
  schedulePickerInjection();
};

const preparePageClose = async () => {
  persistBrowserState();
  pickerRetryTimers.forEach((timer) => window.clearTimeout(timer));
  pickerRetryTimers = [];
  isElementPickerActive.value = false;
  await applyElementPickerState().catch((error) => {
    console.warn('Browser element picker cleanup failed:', error);
  });
};

const elementPickerCleanupScript = () => `
(() => {
  const overlayId = 'nova-real-element-picker-overlay';
  const styleId = 'nova-real-element-picker-style';
  const cleanupKey = '__novaElementPickerCleanup';
  if (typeof window[cleanupKey] === 'function') {
    window[cleanupKey]();
  }
  document.getElementById(overlayId)?.remove();
  document.getElementById(styleId)?.remove();
  window[cleanupKey] = undefined;
})();
`;

const cleanupLegacyElementPickerOverlays = async () => {
  const script = elementPickerCleanupScript();
  await evalBrowserScript(script);
  try {
    const frameTreeResult = await callDevtools('Page.getFrameTree', {});
    const frames = flattenBrowserFrameTree(frameTreeResult.frameTree).slice(1, 35);
    await Promise.allSettled(
      frames.map(async (frame) => {
        const contextId = await createFrameExecutionContext(frame.frameId);
        if (typeof contextId !== 'number') return;
        await callDevtools('Runtime.evaluate', {
          contextId,
          expression: script,
          returnByValue: true,
          awaitPromise: false,
        });
      }),
    );
  } catch (error) {
    console.warn('Browser legacy element picker cleanup failed:', error);
  }
};

const setNativeInspectMode = async (enabled: boolean) => {
  if (enabled) {
    await callDevtools('DOM.enable', {});
    await callDevtools('Overlay.enable', {});
    await callDevtools('Overlay.setInspectMode', {
      mode: 'searchForNode',
      highlightConfig: {
        showInfo: false,
        showStyles: false,
        showAccessibilityInfo: false,
        contentColor: { r: 10, g: 132, b: 255, a: 0.18 },
        paddingColor: { r: 10, g: 132, b: 255, a: 0.14 },
        borderColor: { r: 10, g: 132, b: 255, a: 0.98 },
        marginColor: { r: 10, g: 132, b: 255, a: 0.08 },
        eventTargetColor: { r: 10, g: 132, b: 255, a: 0.18 },
        shapeColor: { r: 10, g: 132, b: 255, a: 0.18 },
        shapeMarginColor: { r: 255, g: 255, b: 255, a: 0.78 },
      },
    });
    return;
  }

  await callDevtools('Overlay.setInspectMode', {
    mode: 'none',
    highlightConfig: {},
  }).catch(() => undefined);
  await callDevtools('Overlay.disable', {}).catch(() => undefined);
};

const applyElementPickerState = async () => {
  if (!pageWebview.value || !isPageWebviewReady.value) return;
  await cleanupLegacyElementPickerOverlays();
  try {
    await setNativeInspectMode(isElementPickerActive.value);
  } catch (error) {
    console.warn('Browser native inspect mode failed:', error);
    isElementPickerActive.value = false;
  }
};

schedulePickerInjection = () => {
  pickerRetryTimers.forEach((timer) => window.clearTimeout(timer));
  pickerRetryTimers = [];
  if (!isElementPickerActive.value) return;
  [0, 120, 350, 900, 1800].forEach((delay) => {
    pickerRetryTimers.push(window.setTimeout(() => void applyElementPickerState(), delay));
  });
};

const {
  listenBrowserAutomationCommands,
} = useBrowserAutomationCommands({
  conversationId: () => browserConversationId.value,
  rawConversationId: () => conversationId,
  currentUrl,
  addressInput,
  history,
  historyIndex,
  isLoading,
  zoomPercent,
  isBrowserWindowReady: isPageWebviewReady,
  canGoBack,
  canGoForward,
  visit,
  ensureBrowserWindowReady: focusPageWebview,
  evalBrowserScript,
  closeNativeBrowserWindow: closePageWebview,
  clearBrowsingData,
  updateBrowserSessionUrl,
  setElementPickerActive: (value) => {
    isElementPickerActive.value = value;
  },
  captureBrowserSnapshot,
  clickSnapshotRef,
  typeSnapshotRef,
});

watch(zoomPercent, (value) => {
  void applyZoom(value).catch((error) => {
    console.warn('Browser zoom failed:', error);
  });
});

watch(currentUrl, () => {
  persistBrowserState();
  void updateBrowserSessionUrl();
});

watch([addressInput, history, historyIndex, zoomPercent], () => {
  persistBrowserState();
});

watch(isMenuOpen, () => {
  void nextTick().then(syncPageWebviewBounds);
});

const handleWindowResize = () => {
  void syncPageWebviewBounds();
};

onMounted(() => {
  void (async () => {
    applyBrowserState(await loadBrowserTabState(conversationId));
    resizeObserver = new ResizeObserver(() => {
      void syncPageWebviewBounds();
    });
    if (browserHost.value) {
      resizeObserver.observe(browserHost.value);
    }
    window.addEventListener('resize', handleWindowResize);
    unlistenBrowserCommand = await listenBrowserAutomationCommands().catch((error) => {
      console.warn('Browser automation listener failed:', error);
      return null;
    });
    await registerBrowserSession();
    if (currentUrl.value) {
      void navigatePageWebview(currentUrl.value);
    }
  })();
});

onBeforeUnmount(() => {
  unlistenBrowserCommand?.();
  void unregisterBrowserSession();
  resizeObserver?.disconnect();
  window.removeEventListener('resize', handleWindowResize);
  void preparePageClose().finally(() => {
    void closePageWebview();
  });
});
</script>

<template>
  <div class="flex h-screen min-h-0 flex-col bg-white text-[#1f2328] dark:bg-[#1e1e1e] dark:text-[#ececec]">
    <div class="flex h-[46px] shrink-0 items-center gap-2 border-b border-[#ececec] bg-white px-4 dark:border-[#313131] dark:bg-[#1e1e1e]">
      <button class="browser-nav-button" :disabled="!canGoBack" title="后退" @click="goBack">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9">
          <path d="M15 18l-6-6 6-6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <button class="browser-nav-button" :disabled="!canGoForward" title="前进" @click="goForward">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9">
          <path d="M9 18l6-6-6-6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <button class="browser-nav-button" :disabled="!currentUrl" title="刷新" @click="reload">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9">
          <path d="M21 12a9 9 0 1 1-2.64-6.36" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M21 3v6h-6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>

      <form class="relative flex min-w-0 flex-1" @submit.prevent="submitAddress">
        <input
          v-model="addressInput"
          class="h-8 min-w-0 flex-1 rounded-xl border border-[#e2e4e7] bg-white px-3 pr-9 text-[14px] text-[#22262b] outline-none transition placeholder:text-[#a8adb4] focus:border-[#c9ccd1] focus:shadow-[0_0_0_3px_rgba(0,0,0,0.03)] dark:border-[#3a3a3a] dark:bg-[#262626] dark:text-[#f2f2f2]"
          placeholder="输入 URL"
        />
        <button
          type="submit"
          class="absolute right-1.5 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-lg text-[#8f949b] transition hover:bg-[#f4f4f4] hover:text-[#30343a] dark:hover:bg-[#333]"
          title="访问"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M7 17L17 7" />
            <path d="M9 7h8v8" />
          </svg>
        </button>
      </form>

      <button class="browser-tool-button" :disabled="!currentUrl" title="聚焦页面" @click="focusPageFromToolbar">
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="M8 3H5a2 2 0 0 0-2 2v3" />
          <path d="M16 3h3a2 2 0 0 1 2 2v3" />
          <path d="M8 21H5a2 2 0 0 1-2-2v-3" />
          <path d="M16 21h3a2 2 0 0 0 2-2v-3" />
          <circle cx="12" cy="12" r="2.5" />
        </svg>
      </button>

      <button
        class="browser-tool-button"
        :class="{ 'browser-annotating-button': isElementPickerActive }"
        :disabled="!currentUrl && !addressInput.trim()"
        :title="isElementPickerActive ? '正在注释，点击退出' : '选择页面元素'"
        @click="toggleElementPicker"
      >
        <span v-if="isElementPickerActive" class="browser-annotating-icon">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 5v14M5 12h14" stroke-linecap="round" />
          </svg>
        </span>
        <span v-if="isElementPickerActive">正在注释</span>
        <svg v-else width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 4v4" />
          <path d="M12 16v4" />
          <path d="M4 12h4" />
          <path d="M16 12h4" />
          <circle cx="12" cy="12" r="3.5" />
        </svg>
      </button>

      <button
        class="browser-menu-button"
        :class="{ 'bg-[#f1f1f1] text-[#24282d] dark:bg-[#303030] dark:text-[#f3f3f3]': isMenuOpen }"
        title="更多"
        @click="isMenuOpen = !isMenuOpen"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round">
          <path d="M12 5h.01M12 12h.01M12 19h.01" />
        </svg>
      </button>
    </div>

    <div v-if="isMenuOpen" class="browser-menu-strip">
      <button class="browser-menu-item" :disabled="!currentUrl" @click="forceReload">强制重新加载</button>
      <div class="browser-zoom-row">
        <span>缩放</span>
        <div class="browser-zoom-control">
          <button class="browser-zoom-button" @click="changeZoom(-10)">-</button>
          <span>{{ zoomPercent }}%</span>
          <button class="browser-zoom-button" @click="changeZoom(10)">+</button>
        </div>
        <button class="browser-cache-reload" @click="zoomPercent = 100">↻</button>
      </div>
      <button class="browser-menu-item" :disabled="!currentUrl" @click="clearCookies">清除 Cookie</button>
      <button class="browser-menu-item" :disabled="!currentUrl" @click="clearCache">清除缓存</button>
      <button class="browser-menu-item" :disabled="!currentUrl" @click="openExternal">外部打开</button>
      <button class="browser-menu-item" @click="closeBrowserShell">关闭窗口</button>
    </div>

    <div class="relative min-h-0 flex-1 bg-white dark:bg-[#171717]">
      <div
        v-if="isLoading"
        class="absolute left-4 top-4 z-10 rounded-full border border-[#e2e4e7] bg-white/90 px-3 py-1 text-xs text-[#7b8188] shadow-sm dark:border-[#353535] dark:bg-[#242424] dark:text-[#aaa29a]"
      >
        加载中...
      </div>
      <div
        v-if="isElementPickerActive"
        class="absolute left-1/2 top-4 z-10 -translate-x-1/2 rounded-full border border-[#d8dde3] bg-white/95 px-3 py-1 text-xs text-[#6d737a] shadow-sm dark:border-[#353535] dark:bg-[#242424] dark:text-[#aaa29a]"
      >
        元素选择模式
      </div>
      <div ref="browserHost" class="h-full w-full bg-white dark:bg-[#171717]" />
    </div>
  </div>
</template>

<style scoped>
.browser-nav-button,
.browser-tool-button,
.browser-menu-button {
  display: flex;
  height: 1.875rem;
  width: 1.875rem;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  color: #a4a9af;
  transition: background-color 0.15s ease, color 0.15s ease, opacity 0.15s ease;
}

.browser-nav-button:hover:not(:disabled),
.browser-tool-button:hover:not(:disabled),
.browser-menu-button:hover:not(:disabled) {
  background: #f3f4f5;
  color: #24282d;
}

.browser-nav-button:disabled,
.browser-tool-button:disabled,
.browser-menu-button:disabled {
  cursor: not-allowed;
  opacity: 0.34;
}

.browser-annotating-icon {
  display: flex;
  height: 1.25rem;
  width: 1.25rem;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  border: 1.5px solid currentColor;
}

.browser-tool-button.browser-annotating-button {
  width: auto;
  min-width: 0;
  gap: 0.45rem;
  border-radius: 0.7rem;
  background: #e5f1ff;
  padding: 0 0.75rem;
  color: #0a84ff;
  font-size: 0.875rem;
  font-weight: 600;
  white-space: nowrap;
}

.browser-menu-strip {
  display: flex;
  min-height: 3.2rem;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  border-bottom: 1px solid #e7e2d7;
  background: #fffdf9;
  padding: 0.55rem 0.85rem;
  color: #27231f;
}

.browser-menu-item {
  display: inline-flex;
  min-height: 2rem;
  align-items: center;
  border-radius: 0.7rem;
  padding: 0 0.7rem;
  color: #2a2520;
  font-size: 0.875rem;
  transition: background-color 0.15s ease, color 0.15s ease;
}

.browser-menu-item:hover:not(:disabled) {
  background: #f1ece4;
}

.browser-menu-item:disabled {
  cursor: not-allowed;
  color: #aaa39b;
}

.browser-zoom-row {
  display: flex;
  min-height: 2rem;
  align-items: center;
  gap: 0.5rem;
  padding: 0 0.25rem;
  font-size: 0.875rem;
}

.browser-zoom-control {
  display: flex;
  height: 2rem;
  align-items: center;
  overflow: hidden;
  border: 1px solid #ded8cf;
  border-radius: 0.65rem;
  background: #f8f5f0;
}

.browser-zoom-control span {
  display: inline-flex;
  min-width: 3.25rem;
  align-items: center;
  justify-content: center;
  border-inline: 1px solid #ded8cf;
  font-weight: 600;
}

.browser-zoom-button,
.browser-cache-reload {
  display: flex;
  height: 2rem;
  width: 2rem;
  align-items: center;
  justify-content: center;
  color: #918a82;
  transition: color 0.15s ease, background-color 0.15s ease;
}

.browser-zoom-button:hover,
.browser-cache-reload:hover {
  background: #eee8df;
  color: #35302a;
}
</style>
