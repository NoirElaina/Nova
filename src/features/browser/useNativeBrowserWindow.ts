import { ref, shallowRef, type Ref } from 'vue';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';

type UseNativeBrowserWindowOptions = {
  browserLabel: string;
  conversationId: () => string | null;
  currentUrl: Ref<string>;
  isLoading: Ref<boolean>;
  shouldAutoRestore: () => boolean;
};

const BROWSER_WINDOW_WIDTH = 1180;
const BROWSER_WINDOW_HEIGHT = 760;

export function useNativeBrowserWindow(options: UseNativeBrowserWindowOptions) {
  const browserWindowLabel = `nova-browser-window-${options.browserLabel}`;
  const browserWindow = shallowRef<WebviewWindow | null>(null);
  const isBrowserWindowReady = ref(false);
  let restoreRetryTimer: number | null = null;
  let unlistenDestroyed: UnlistenFn | null = null;

  const clearQueuedRestore = () => {
    if (restoreRetryTimer === null) return;
    window.clearTimeout(restoreRetryTimer);
    restoreRetryTimer = null;
  };

  const clearDestroyedListener = () => {
    const unlisten = unlistenDestroyed;
    unlistenDestroyed = null;
    if (!unlisten) return;
    void Promise.resolve(unlisten()).catch((error) => {
      console.warn('Browser window destroyed listener cleanup failed:', error);
    });
  };

  const markBrowserWindowClosed = () => {
    clearQueuedRestore();
    clearDestroyedListener();
    browserWindow.value = null;
    isBrowserWindowReady.value = false;
    options.isLoading.value = false;
  };

  const bindBrowserWindowLifecycle = (windowRef: WebviewWindow) => {
    clearDestroyedListener();
    void windowRef
      .once('tauri://destroyed', () => {
        markBrowserWindowClosed();
      })
      .then((unlisten) => {
        unlistenDestroyed = unlisten;
      })
      .catch((error) => {
        console.warn('Browser window destroyed listener failed:', error);
      });
  };

  const showAndFocusBrowserWindow = async (windowRef: WebviewWindow) => {
    await windowRef.show().catch((error) => {
      console.warn('Browser window show failed:', error);
    });
    await windowRef.setFocus().catch((error) => {
      console.warn('Browser window focus failed:', error);
    });
  };

  const buildBrowserWindowUrl = (initialUrl?: string) => {
    const url = new URL(window.location.href);
    url.search = '';
    url.hash = '';
    url.searchParams.set('novaBrowserWindow', '1');
    url.searchParams.set('pageLabel', options.browserLabel);
    const conversationId = options.conversationId();
    if (conversationId) {
      url.searchParams.set('conversationId', conversationId);
    }
    if (initialUrl) {
      url.searchParams.set('initialUrl', initialUrl);
    }
    return url.toString();
  };

  const reuseExistingBrowserWindow = async () => {
    const existing = await WebviewWindow.getByLabel(browserWindowLabel).catch(() => null);
    if (!existing) return false;

    browserWindow.value = existing;
    isBrowserWindowReady.value = true;
    bindBrowserWindowLifecycle(existing);
    await showAndFocusBrowserWindow(existing);
    options.isLoading.value = false;
    return true;
  };

  const createBrowserWindow = async (initialUrl = ''): Promise<boolean> => {
    clearQueuedRestore();
    if (browserWindow.value && !isBrowserWindowReady.value) return false;
    if (await reuseExistingBrowserWindow()) return true;

    const windowRef = new WebviewWindow(browserWindowLabel, {
      url: buildBrowserWindowUrl(initialUrl),
      title: 'Nova Browser',
      width: BROWSER_WINDOW_WIDTH,
      height: BROWSER_WINDOW_HEIGHT,
      minWidth: 640,
      minHeight: 420,
      center: true,
      preventOverflow: true,
      resizable: true,
      focus: true,
      visible: true,
      devtools: true,
    });

    browserWindow.value = windowRef;
    isBrowserWindowReady.value = false;
    bindBrowserWindowLifecycle(windowRef);

    void windowRef.once('tauri://error', (event) => {
      console.error('Browser window failed to create:', event.payload);
      markBrowserWindowClosed();
    });
    void windowRef.once('tauri://created', () => {
      isBrowserWindowReady.value = true;
      options.isLoading.value = false;
    });
    return true;
  };

  const focusBrowserWindow = async (): Promise<boolean> => {
    const windowRef = browserWindow.value;
    if (windowRef && isBrowserWindowReady.value) {
      await showAndFocusBrowserWindow(windowRef);
      return true;
    }
    return createBrowserWindow(options.currentUrl.value);
  };

  const closeBrowserWindow = async () => {
    clearQueuedRestore();
    const windowRef = browserWindow.value;
    if (!windowRef) {
      markBrowserWindowClosed();
      return;
    }
    try {
      await windowRef.close();
    } catch (error) {
      console.warn('Browser window close failed:', error);
    } finally {
      markBrowserWindowClosed();
    }
  };

  const navigateBrowserWindow = async (url: string): Promise<boolean> => {
    if (!browserWindow.value) {
      return createBrowserWindow(url);
    }
    if (!isBrowserWindowReady.value) return false;
    await focusBrowserWindow();
    return true;
  };

  const reopenSavedBrowserWindow = async () => {
    if (!options.shouldAutoRestore()) return;
    options.isLoading.value = true;
    const opened = await createBrowserWindow(options.currentUrl.value);
    if (!opened) {
      options.isLoading.value = false;
      queueReopenSavedBrowserWindow();
    }
  };

  const queueReopenSavedBrowserWindow = () => {
    if (!options.shouldAutoRestore()) return;
    if (browserWindow.value || restoreRetryTimer !== null) return;
    restoreRetryTimer = window.setTimeout(() => {
      restoreRetryTimer = null;
      void reopenSavedBrowserWindow();
    }, 120);
  };

  return {
    browserWindow,
    isBrowserWindowReady,
    closeBrowserWindow,
    navigateBrowserWindow,
    reopenSavedBrowserWindow,
    queueReopenSavedBrowserWindow,
    clearQueuedRestore,
    focusBrowserWindow,
  };
}
