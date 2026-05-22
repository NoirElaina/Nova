import { nextTick, ref, shallowRef, type Ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';
import { Webview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';

type UseBrowserPageWebviewOptions = {
  pageLabel: string;
  browserHost: Ref<HTMLElement | null>;
  currentUrl: Ref<string>;
  isLoading: Ref<boolean>;
  zoomPercent: Ref<number>;
  onReady?: () => void;
  onAfterNavigate?: () => void;
};

export function useBrowserPageWebview(options: UseBrowserPageWebviewOptions) {
  const pageWebview = shallowRef<Webview | null>(null);
  const isPageWebviewReady = ref(false);

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

  const syncPageWebviewBounds = async () => {
    const webview = pageWebview.value;
    if (!webview || !isPageWebviewReady.value) return;
    const rect = getHostBounds();
    if (!rect) {
      await webview.hide().catch((error) => {
        console.warn('Browser page webview hide failed:', error);
      });
      return;
    }
    await webview.setPosition(new LogicalPosition(Math.round(rect.left), Math.round(rect.top)));
    await webview.setSize(new LogicalSize(Math.round(rect.width), Math.round(rect.height)));
    await webview.show();
  };

  const reuseExistingPageWebview = async (url: string) => {
    const existing = await Webview.getByLabel(options.pageLabel).catch(() => null);
    if (!existing) return false;
    pageWebview.value = existing;
    isPageWebviewReady.value = true;
    await syncPageWebviewBounds();
    await existing.setZoom(options.zoomPercent.value / 100).catch((error) => {
      console.warn('Browser zoom failed:', error);
    });
    await invoke('browser_navigate_window', { label: options.pageLabel, url }).catch((error) => {
      console.warn('Browser navigation failed:', error);
    });
    options.isLoading.value = false;
    options.onReady?.();
    return true;
  };

  const createPageWebview = async (url: string): Promise<boolean> => {
    const rect = await waitForHostBounds();
    if (!rect) return false;
    if (pageWebview.value && !isPageWebviewReady.value) return false;
    if (await reuseExistingPageWebview(url)) return true;

    const child = new Webview(getCurrentWindow(), options.pageLabel, {
      url,
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      focus: true,
    });

    pageWebview.value = child;
    isPageWebviewReady.value = false;
    void child.once('tauri://error', (event) => {
      console.error('Browser page webview failed to create:', event.payload);
      pageWebview.value = null;
      isPageWebviewReady.value = false;
      options.isLoading.value = false;
    });
    void child.once('tauri://created', () => {
      isPageWebviewReady.value = true;
      options.isLoading.value = false;
      void child.setZoom(options.zoomPercent.value / 100).catch((error) => {
        console.warn('Browser zoom failed:', error);
      });
      void syncPageWebviewBounds();
      options.onReady?.();
    });
    return true;
  };

  const navigatePageWebview = async (url: string): Promise<boolean> => {
    if (!pageWebview.value) {
      return createPageWebview(url);
    }
    if (!isPageWebviewReady.value) return false;

    await syncPageWebviewBounds();
    await invoke('browser_navigate_window', { label: options.pageLabel, url }).catch((error) => {
      console.warn('Browser navigation failed:', error);
    });
    window.setTimeout(() => {
      options.isLoading.value = false;
      options.onAfterNavigate?.();
    }, 900);
    return true;
  };

  const focusPageWebview = async (): Promise<boolean> => {
    if (pageWebview.value && isPageWebviewReady.value) {
      await syncPageWebviewBounds();
      await pageWebview.value.setFocus().catch((error) => {
        console.warn('Browser page focus failed:', error);
      });
      return true;
    }
    if (!options.currentUrl.value) return false;
    return createPageWebview(options.currentUrl.value);
  };

  const closePageWebview = async () => {
    const webview = pageWebview.value;
    pageWebview.value = null;
    isPageWebviewReady.value = false;
    options.isLoading.value = false;
    if (!webview) return;
    await webview.close().catch((error) => {
      console.warn('Browser page close failed:', error);
    });
  };

  const evalBrowserScript = async (script: string) => {
    if (!pageWebview.value || !isPageWebviewReady.value) return;
    await invoke('browser_eval_window_script', { label: options.pageLabel, script }).catch((error) => {
      console.warn('Browser script evaluation failed:', error);
    });
  };

  const reloadPageWebview = async () => {
    await invoke('browser_reload_window', { label: options.pageLabel });
  };

  const clearBrowsingData = async () => {
    if (!isPageWebviewReady.value) return;
    await pageWebview.value?.clearAllBrowsingData();
  };

  const applyZoom = async (value: number) => {
    if (!isPageWebviewReady.value) return;
    await pageWebview.value?.setZoom(value / 100);
  };

  return {
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
  };
}
