<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';
import { Webview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openUrl } from '@tauri-apps/plugin-opener';

const addressInput = ref('');
const currentUrl = ref('');
const history = ref<string[]>([]);
const historyIndex = ref(-1);
const isLoading = ref(false);
const isMenuOpen = ref(false);
const zoomPercent = ref(100);
const showDeviceToolbar = ref(false);
const isElementPickerActive = ref(false);
const browserHost = ref<HTMLElement | null>(null);
const browserLabel = `nova-browser-${crypto.randomUUID()}`;
let browserWebview: Webview | null = null;
let isBrowserWebviewReady = false;
let resizeObserver: ResizeObserver | null = null;
let pickerRetryTimers: number[] = [];

const canGoBack = computed(() => historyIndex.value > 0);
const canGoForward = computed(() => historyIndex.value >= 0 && historyIndex.value < history.value.length - 1);

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

const visit = (raw: string, pushHistory = true) => {
  const nextUrl = normalizeAddress(raw);
  if (!nextUrl) return;

  currentUrl.value = nextUrl;
  addressInput.value = nextUrl;
  isLoading.value = true;
  void navigateNativeWebview(nextUrl);

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
  if (!browserWebview || !isBrowserWebviewReady) {
    void navigateNativeWebview(currentUrl.value);
    return;
  }
  isLoading.value = true;
  void invoke('browser_reload_webview', { label: browserLabel })
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
};

const forceReload = () => {
  reload();
  isMenuOpen.value = false;
};

const changeZoom = (delta: number) => {
  zoomPercent.value = Math.min(200, Math.max(50, zoomPercent.value + delta));
};

const clearCookies = () => {
  if (isBrowserWebviewReady) {
    void browserWebview?.clearAllBrowsingData().catch((error) => {
      console.warn('Browser clear browsing data failed:', error);
    });
  }
  isMenuOpen.value = false;
};

const clearCache = () => {
  if (isBrowserWebviewReady) {
    void browserWebview?.clearAllBrowsingData().catch((error) => {
      console.warn('Browser clear cache failed:', error);
    });
  }
  isLoading.value = Boolean(currentUrl.value);
  reload();
  isMenuOpen.value = false;
};

const capturePage = () => {
  isMenuOpen.value = false;
};

const toggleElementPicker = () => {
  isElementPickerActive.value = !isElementPickerActive.value;
  isMenuOpen.value = false;
  void applyElementPickerState();
};

const getHostBounds = () => {
  const host = browserHost.value;
  if (!host) return null;
  const rect = host.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  return rect;
};

const syncWebviewBounds = async () => {
  if (!browserWebview || !isBrowserWebviewReady) return;
  try {
    const rect = getHostBounds();
    if (!rect || !currentUrl.value) {
      await browserWebview.hide();
      return;
    }

    await browserWebview.setPosition(new LogicalPosition(Math.round(rect.left), Math.round(rect.top)));
    await browserWebview.setSize(new LogicalSize(Math.round(rect.width), Math.round(rect.height)));
    await browserWebview.show();
  } catch (error) {
    console.warn('Browser webview bounds sync failed:', error);
  }
};

const closeNativeWebview = async () => {
  if (!browserWebview || !isBrowserWebviewReady) {
    browserWebview = null;
    isBrowserWebviewReady = false;
    return;
  }
  try {
    await browserWebview.close();
  } catch (error) {
    console.warn('Browser webview close failed:', error);
  } finally {
    browserWebview = null;
    isBrowserWebviewReady = false;
  }
};

const createNativeWebview = async (url: string) => {
  await nextTick();
  const rect = getHostBounds();
  if (!rect) return;
  if (browserWebview && !isBrowserWebviewReady) return;

  const child = new Webview(getCurrentWindow(), browserLabel, {
    url,
    x: Math.round(rect.left),
    y: Math.round(rect.top),
    width: Math.round(rect.width),
    height: Math.round(rect.height),
    focus: true,
    devtools: true,
  });

  browserWebview = child;
  isBrowserWebviewReady = false;
  void child.once('tauri://error', (event) => {
    console.error('Browser webview failed to create:', event.payload);
    browserWebview = null;
    isBrowserWebviewReady = false;
    isLoading.value = false;
  });
  void child.once('tauri://created', () => {
    isBrowserWebviewReady = true;
    isLoading.value = false;
    void child.setZoom(zoomPercent.value / 100).catch((error) => {
      console.warn('Browser zoom failed:', error);
    });
    void syncWebviewBounds();
    schedulePickerInjection();
  });
};

const navigateNativeWebview = async (url: string) => {
  await nextTick();
  if (!browserWebview) {
    await createNativeWebview(url);
    return;
  }
  if (!isBrowserWebviewReady) return;

  await syncWebviewBounds();
  await invoke('browser_navigate_webview', { label: browserLabel, url }).catch((error) => {
    console.warn('Browser navigation failed:', error);
  });
  window.setTimeout(() => {
    isLoading.value = false;
    schedulePickerInjection();
  }, 900);
};

const evalBrowserScript = async (script: string) => {
  if (!browserWebview || !isBrowserWebviewReady) return;
  await invoke('browser_eval_webview_script', { label: browserLabel, script }).catch((error) => {
    console.warn('Browser script evaluation failed:', error);
  });
};

const elementPickerScript = (enabled: boolean) => `
(() => {
  const overlayId = 'nova-real-element-picker-overlay';
  const cleanupKey = '__novaElementPickerCleanup';
  if (typeof window[cleanupKey] === 'function') {
    window[cleanupKey]();
  }
  if (!${enabled ? 'true' : 'false'}) {
    return;
  }

  const overlay = document.createElement('div');
  overlay.id = overlayId;
  Object.assign(overlay.style, {
    position: 'fixed',
    left: '0px',
    top: '0px',
    width: '0px',
    height: '0px',
    zIndex: '2147483647',
    pointerEvents: 'none',
    border: '2px solid #0a84ff',
    borderRadius: '8px',
    background: 'rgba(10, 132, 255, 0.18)',
    boxShadow: '0 0 0 1px rgba(255,255,255,0.78), 0 10px 30px rgba(10,132,255,0.18)',
    transition: 'left 80ms ease, top 80ms ease, width 80ms ease, height 80ms ease',
  });
  document.documentElement.appendChild(overlay);

  let currentElement = null;
  const hiddenTags = new Set(['HTML', 'BODY']);
  const selectableElementFromPoint = (x, y) => {
    let element = document.elementFromPoint(x, y);
    if (!element || element === overlay) return null;
    if (element.nodeType !== Node.ELEMENT_NODE) return null;
    if (hiddenTags.has(element.tagName) && element.children.length) {
      const children = Array.from(element.children);
      element = children.find((child) => {
        const rect = child.getBoundingClientRect();
        return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      }) || element;
    }
    return element;
  };

  const paint = (element) => {
    if (!element) return;
    const rect = element.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    currentElement = element;
    overlay.style.left = Math.max(0, rect.left) + 'px';
    overlay.style.top = Math.max(0, rect.top) + 'px';
    overlay.style.width = rect.width + 'px';
    overlay.style.height = rect.height + 'px';
  };

  const onMove = (event) => {
    paint(selectableElementFromPoint(event.clientX, event.clientY));
  };
  const onClick = (event) => {
    event.preventDefault();
    event.stopPropagation();
    paint(selectableElementFromPoint(event.clientX, event.clientY));
  };
  const onScroll = () => {
    if (currentElement) paint(currentElement);
  };
  const cleanup = () => {
    document.removeEventListener('mousemove', onMove, true);
    document.removeEventListener('click', onClick, true);
    document.removeEventListener('scroll', onScroll, true);
    window.removeEventListener('resize', onScroll, true);
    overlay.remove();
    window[cleanupKey] = undefined;
  };

  document.addEventListener('mousemove', onMove, true);
  document.addEventListener('click', onClick, true);
  document.addEventListener('scroll', onScroll, true);
  window.addEventListener('resize', onScroll, true);
  window[cleanupKey] = cleanup;
})();
`;

const applyElementPickerState = async () => {
  await evalBrowserScript(elementPickerScript(isElementPickerActive.value));
};

const schedulePickerInjection = () => {
  pickerRetryTimers.forEach((timer) => window.clearTimeout(timer));
  pickerRetryTimers = [];
  if (!isElementPickerActive.value) return;
  [250, 900, 1800].forEach((delay) => {
    pickerRetryTimers.push(window.setTimeout(() => void applyElementPickerState(), delay));
  });
};

const handleWindowResize = () => {
  void syncWebviewBounds();
};

const openBrowserMenu = async (event: MouseEvent) => {
  event.stopPropagation();
  isMenuOpen.value = !isMenuOpen.value;
};

watch(zoomPercent, (value) => {
  if (!isBrowserWebviewReady) return;
  void browserWebview?.setZoom(value / 100).catch((error) => {
    console.warn('Browser zoom failed:', error);
  });
});

watch(showDeviceToolbar, () => {
  void nextTick().then(syncWebviewBounds);
});

watch(browserHost, (host, oldHost) => {
  if (oldHost) {
    resizeObserver?.unobserve(oldHost);
  }
  if (host) {
    resizeObserver?.observe(host);
  }
  void syncWebviewBounds();
});

onMounted(() => {
  resizeObserver = new ResizeObserver(() => {
    void syncWebviewBounds();
  });
  if (browserHost.value) {
    resizeObserver.observe(browserHost.value);
  }
  window.addEventListener('resize', handleWindowResize);
});

onBeforeUnmount(() => {
  pickerRetryTimers.forEach((timer) => window.clearTimeout(timer));
  resizeObserver?.disconnect();
  window.removeEventListener('resize', handleWindowResize);
  isElementPickerActive.value = false;
  void applyElementPickerState().finally(() => {
    void closeNativeWebview();
  });
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-col bg-white text-[#1f2328] dark:bg-[#1e1e1e] dark:text-[#ececec]">
    <div class="flex h-[46px] shrink-0 items-center gap-1.5 border-b border-[#ececec] px-4 dark:border-[#313131]">
      <button
        class="browser-nav-button"
        :disabled="!canGoBack"
        title="后退"
        @click="goBack"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9">
          <path d="M15 18l-6-6 6-6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <button
        class="browser-nav-button"
        :disabled="!canGoForward"
        title="前进"
        @click="goForward"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9">
          <path d="M9 18l6-6-6-6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <button
        class="browser-nav-button"
        :disabled="!currentUrl"
        title="刷新"
        @click="reload"
      >
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

      <button
        class="browser-tool-button"
        title="截图"
        @click="capturePage"
      >
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

      <div class="relative">
        <button
          class="browser-menu-button"
          :class="{ 'bg-[#f1f1f1] text-[#24282d] dark:bg-[#303030] dark:text-[#f3f3f3]': isMenuOpen }"
          title="更多"
          @click="openBrowserMenu"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round">
            <path d="M12 5h.01M12 12h.01M12 19h.01" />
          </svg>
        </button>

        <div
          v-if="isMenuOpen"
          class="browser-menu-panel"
          @click.stop
        >
          <button class="browser-menu-item" :disabled="!currentUrl" @click="forceReload">
            强制重新加载
          </button>
          <button
            class="browser-menu-item"
            @click="showDeviceToolbar = !showDeviceToolbar; isMenuOpen = false"
          >
            {{ showDeviceToolbar ? '隐藏设备工具栏' : '显示设备工具栏' }}
          </button>
          <div class="browser-menu-separator" />
          <div class="browser-zoom-row">
            <span>缩放</span>
            <div class="browser-zoom-control">
              <button class="browser-zoom-button" @click="changeZoom(-10)">−</button>
              <span>{{ zoomPercent }}%</span>
              <button class="browser-zoom-button" @click="changeZoom(10)">＋</button>
            </div>
            <button class="browser-cache-reload" @click="zoomPercent = 100">↻</button>
          </div>
          <div class="browser-menu-separator" />
          <button class="browser-menu-item" :disabled="!currentUrl" @click="clearCookies">
            清除 Cookie
          </button>
          <button class="browser-menu-item" :disabled="!currentUrl" @click="clearCache">
            清除缓存
          </button>
          <button class="browser-menu-item" :disabled="!currentUrl" @click="openExternal(); isMenuOpen = false">
            外部打开
          </button>
        </div>
      </div>
    </div>

    <div v-if="showDeviceToolbar" class="flex h-9 shrink-0 items-center justify-center border-b border-[#ececec] bg-[#fbfbfb] text-xs text-[#8b8f96] dark:border-[#313131] dark:bg-[#202020]">
      设备工具栏：{{ Math.round(390 * zoomPercent / 100) }} x {{ Math.round(844 * zoomPercent / 100) }}
    </div>

    <div v-if="!currentUrl" class="min-h-0 flex-1 bg-white dark:bg-[#171717]" />

    <div
      v-else
      class="relative min-h-0 flex-1 overflow-auto bg-white dark:bg-[#171717]"
      @scroll="syncWebviewBounds"
      @click="isMenuOpen = false"
    >
      <div
        v-if="isLoading"
        class="absolute left-5 top-5 z-10 rounded-full border border-[#e2e4e7] bg-white/90 px-3 py-1 text-xs text-[#7b8188] shadow-sm dark:border-[#353535] dark:bg-[#242424] dark:text-[#aaa29a]"
      >
        加载中...
      </div>
      <div
        v-if="isElementPickerActive"
        class="absolute left-1/2 top-4 z-10 -translate-x-1/2 rounded-full border border-[#d8dde3] bg-white/95 px-3 py-1 text-xs text-[#6d737a] shadow-sm dark:border-[#353535] dark:bg-[#242424] dark:text-[#aaa29a]"
      >
        元素选择模式
      </div>
      <div
        ref="browserHost"
        class="origin-top-left border-0 bg-white"
        :class="showDeviceToolbar ? 'mx-auto h-[844px] w-[390px] shadow-[0_0_0_1px_rgba(0,0,0,0.08)]' : 'h-full w-full'"
        @click="isMenuOpen = false"
      />
    </div>
  </div>
</template>

<style scoped>
.browser-nav-button,
.browser-tool-button,
.browser-menu-button {
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  color: #a4a9af;
  transition: background-color 0.15s ease, color 0.15s ease, opacity 0.15s ease;
}

.browser-nav-button,
.browser-tool-button,
.browser-menu-button {
  height: 1.875rem;
  width: 1.875rem;
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

.browser-tool-button.browser-annotating-button:hover {
  background: #dcedff;
  color: #0877e6;
}

.browser-menu-panel {
  position: absolute;
  right: 0;
  top: calc(100% + 0.45rem);
  z-index: 60;
  width: 15.5rem;
  border: 1px solid #ded7cc;
  border-radius: 1rem;
  background: #fffdf9;
  padding: 0.55rem;
  color: #27231f;
  box-shadow: 0 18px 45px rgba(43, 34, 24, 0.16), 0 2px 8px rgba(43, 34, 24, 0.08);
}

.browser-menu-item {
  display: flex;
  width: 100%;
  align-items: center;
  border-radius: 0.7rem;
  padding: 0.55rem 0.7rem;
  color: #2a2520;
  font-size: 0.875rem;
  text-align: left;
  transition: background-color 0.15s ease, color 0.15s ease;
}

.browser-menu-item:hover:not(:disabled) {
  background: #f1ece4;
}

.browser-menu-item:disabled {
  cursor: not-allowed;
  color: #aaa39b;
}

.browser-menu-separator {
  margin: 0.35rem 0;
  height: 1px;
  background: #e5ded4;
}

.browser-zoom-row {
  display: grid;
  min-height: 2rem;
  grid-template-columns: 1fr auto auto;
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
