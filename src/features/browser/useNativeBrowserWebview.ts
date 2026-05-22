import { nextTick, ref, shallowRef, type Ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';
import { Webview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';

type UseNativeBrowserWebviewOptions = {
  browserLabel: string;
  browserHost: Ref<HTMLElement | null>;
  currentUrl: Ref<string>;
  isLoading: Ref<boolean>;
  zoomPercent: Ref<number>;
  isVisible: () => boolean;
  onReady?: () => void;
  onAfterNavigate?: () => void;
};

export function useNativeBrowserWebview(options: UseNativeBrowserWebviewOptions) {
  const browserWebview = shallowRef<Webview | null>(null);
  const isBrowserWebviewReady = ref(false);
  let restoreRetryTimer: number | null = null;

  const getHostBounds = () => {
    const host = options.browserHost.value;
    if (!host) return null;
    const rect = host.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return rect;
  };

  const waitForHostBounds = async (timeoutMs = 1400) => {
    const startedAt = performance.now();
    while (performance.now() - startedAt < timeoutMs) {
      await nextTick();
      const rect = getHostBounds();
      if (rect) return rect;
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    }
    return getHostBounds();
  };

  const syncWebviewBounds = async () => {
    const webview = browserWebview.value;
    if (!webview || !isBrowserWebviewReady.value) return;
    try {
      const rect = getHostBounds();
      if (!rect || !options.currentUrl.value) {
        await webview.hide();
        return;
      }

      await webview.setPosition(new LogicalPosition(Math.round(rect.left), Math.round(rect.top)));
      await webview.setSize(new LogicalSize(Math.round(rect.width), Math.round(rect.height)));
      await webview.show();
    } catch (error) {
      console.warn('Browser webview bounds sync failed:', error);
    }
  };

  const closeNativeWebview = async () => {
    const webview = browserWebview.value;
    if (!webview || !isBrowserWebviewReady.value) {
      browserWebview.value = null;
      isBrowserWebviewReady.value = false;
      return;
    }
    try {
      await webview.close();
    } catch (error) {
      console.warn('Browser webview close failed:', error);
    } finally {
      browserWebview.value = null;
      isBrowserWebviewReady.value = false;
    }
  };

  const clearQueuedRestore = () => {
    if (restoreRetryTimer !== null) {
      window.clearTimeout(restoreRetryTimer);
      restoreRetryTimer = null;
    }
  };

  const hideNativeWebview = async () => {
    clearQueuedRestore();
    const webview = browserWebview.value;
    if (webview && isBrowserWebviewReady.value) {
      await webview.hide().catch((error) => {
        console.warn('Browser webview hide failed:', error);
      });
    }
  };

  const createNativeWebview = async (url: string): Promise<boolean> => {
    await nextTick();
    const rect = await waitForHostBounds();
    if (!rect) return false;
    if (browserWebview.value && !isBrowserWebviewReady.value) return false;

    const child = new Webview(getCurrentWindow(), options.browserLabel, {
      url,
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      focus: true,
      devtools: true,
    });

    browserWebview.value = child;
    isBrowserWebviewReady.value = false;
    void child.once('tauri://error', (event) => {
      console.error('Browser webview failed to create:', event.payload);
      browserWebview.value = null;
      isBrowserWebviewReady.value = false;
      options.isLoading.value = false;
    });
    void child.once('tauri://created', () => {
      isBrowserWebviewReady.value = true;
      options.isLoading.value = false;
      void child.setZoom(options.zoomPercent.value / 100).catch((error) => {
        console.warn('Browser zoom failed:', error);
      });
      void syncWebviewBounds();
      options.onReady?.();
    });
    return true;
  };

  const navigateNativeWebview = async (url: string): Promise<boolean> => {
    await nextTick();
    if (!browserWebview.value) {
      return createNativeWebview(url);
    }
    if (!isBrowserWebviewReady.value) return false;

    await syncWebviewBounds();
    await invoke('browser_navigate_webview', { label: options.browserLabel, url }).catch((error) => {
      console.warn('Browser navigation failed:', error);
    });
    window.setTimeout(() => {
      options.isLoading.value = false;
      options.onAfterNavigate?.();
    }, 900);
    return true;
  };

  const reopenSavedBrowserSurface = async () => {
    if (!options.isVisible()) return;
    if (!options.currentUrl.value) return;
    options.isLoading.value = true;
    const opened = await navigateNativeWebview(options.currentUrl.value);
    if (!opened) {
      options.isLoading.value = false;
      queueReopenSavedBrowserSurface();
    }
  };

  const queueReopenSavedBrowserSurface = () => {
    if (!options.isVisible()) return;
    if (!options.currentUrl.value || browserWebview.value || restoreRetryTimer !== null) return;
    restoreRetryTimer = window.setTimeout(() => {
      restoreRetryTimer = null;
      void reopenSavedBrowserSurface();
    }, 120);
  };

  const evalBrowserScript = async (script: string) => {
    if (!browserWebview.value || !isBrowserWebviewReady.value) return;
    await invoke('browser_eval_webview_script', { label: options.browserLabel, script }).catch((error) => {
      console.warn('Browser script evaluation failed:', error);
    });
  };

  const reloadNativeWebview = async () => {
    await invoke('browser_reload_webview', { label: options.browserLabel });
  };

  const clearBrowsingData = async () => {
    if (!isBrowserWebviewReady.value) return;
    await browserWebview.value?.clearAllBrowsingData();
  };

  const applyZoom = async (value: number) => {
    if (!isBrowserWebviewReady.value) return;
    await browserWebview.value?.setZoom(value / 100);
  };

  return {
    browserWebview,
    isBrowserWebviewReady,
    syncWebviewBounds,
    closeNativeWebview,
    hideNativeWebview,
    navigateNativeWebview,
    reopenSavedBrowserSurface,
    queueReopenSavedBrowserSurface,
    evalBrowserScript,
    reloadNativeWebview,
    clearBrowsingData,
    applyZoom,
    clearQueuedRestore,
  };
}
