import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Ref } from 'vue';
import { clearBrowserTabState } from './browser-tab-state';
import { clickScript, typeScript } from './useBrowserSnapshot';

export type BrowserAutomationCommand = {
  conversationId: string;
  requestId: string;
  action: string;
  payload?: {
    input?: Record<string, unknown>;
  };
};

type UseBrowserAutomationCommandsOptions = {
  conversationId: () => string;
  rawConversationId: () => string | null | undefined;
  currentUrl: Ref<string>;
  addressInput: Ref<string>;
  history: Ref<string[]>;
  historyIndex: Ref<number>;
  isLoading: Ref<boolean>;
  zoomPercent: Ref<number>;
  isBrowserWindowReady: Ref<boolean>;
  canGoBack: Ref<boolean>;
  canGoForward: Ref<boolean>;
  visit: (raw: string, pushHistory?: boolean) => void;
  ensureBrowserWindowReady: () => Promise<boolean>;
  evalBrowserScript: (script: string) => Promise<void>;
  evalBrowserScriptResult?: (script: string) => Promise<any>;
  closeNativeBrowserWindow: () => Promise<void>;
  clearBrowsingData: () => Promise<void>;
  updateBrowserSessionUrl: () => Promise<void>;
  setElementPickerActive: (value: boolean) => void;
  captureBrowserSnapshot: () => Promise<Record<string, unknown>>;
  clickSnapshotRef: (ref: string) => Promise<void>;
  typeSnapshotRef: (ref: string, input: Record<string, unknown>) => Promise<void>;
};

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

// 点击后等待导航稳定的上限（含未导航的点击——href 不变会很快收敛）。
const CLICK_SETTLE_TIMEOUT_MS = 6_000;
// 导航后等待页面稳定的上限。
const NAVIGATE_SETTLE_TIMEOUT_MS = 10_000;
// 判定"稳定"需要连续一致的探测次数 + 间隔。
const SETTLE_STEP_MS = 200;
const SETTLE_STABLE_POLLS = 3;
// snapshot：等待 isLoading 复位的上限与步长（visit/reload 会置位）。
const SNAPSHOT_LOAD_WAIT_MS = 8_000;
const SNAPSHOT_LOAD_STEP_MS = 200;
// snapshot：假空页（text/elements 全空但 URL 存在）重试次数与间隔。
const SNAPSHOT_EMPTY_RETRIES = 3;
const SNAPSHOT_RETRY_STEP_MS = 450;

const isChromeErrorUrl = (url: string) => url.startsWith('chrome-error://');

const automationInput = (command: BrowserAutomationCommand) =>
  command.payload?.input ?? {};

export function useBrowserAutomationCommands(options: UseBrowserAutomationCommandsOptions) {
  const browserStatePayload = (extra: Record<string, unknown> = {}) => ({
    url: options.currentUrl.value || null,
    address: options.addressInput.value || null,
    canGoBack: options.canGoBack.value,
    canGoForward: options.canGoForward.value,
    isLoading: options.isLoading.value,
    zoomPercent: options.zoomPercent.value,
    note:
      'Nova Browser v1 controls the conversation-scoped browser window and can navigate, click, type, reset, and return a bounded DOM summary for the current page.',
    ...extra,
  });

  const reportBrowserCommandResult = async (
    requestId: string,
    ok: boolean,
    result?: Record<string, unknown>,
    error?: string,
  ) => {
    await invoke('browser_automation_result', {
      payload: {
        requestId,
        ok,
        result: result ?? null,
        error: error ?? null,
      },
    }).catch((invokeError) => {
      console.warn('Browser automation result report failed:', invokeError);
    });
  };

  /** 用页面的真实 location.href 同步地址栏/历史/后端会话状态（不信任本地 ref 的滞后值）。 */
  const syncUrlState = (realUrl: string) => {
    if (!realUrl || isChromeErrorUrl(realUrl)) return;
    options.currentUrl.value = realUrl;
    options.addressInput.value = realUrl;
    const nextHistory = options.history.value.slice(0, options.historyIndex.value + 1);
    if (nextHistory[nextHistory.length - 1] !== realUrl) {
      nextHistory.push(realUrl);
      options.history.value = nextHistory;
      options.historyIndex.value = nextHistory.length - 1;
    }
    void options.updateBrowserSessionUrl();
  };

  /** 探测页面当前真实状态：href + readyState。文档替换间隙可能失败，失败返回 null 由调用方轮询。 */
  const probePageState = async (): Promise<{ href: string; readyState: string } | null> => {
    if (!options.evalBrowserScriptResult) return null;
    try {
      const href = await options.evalBrowserScriptResult('location.href');
      if (typeof href !== 'string' || !href) return null;
      const readyState = await options.evalBrowserScriptResult('document.readyState');
      return { href, readyState: typeof readyState === 'string' ? readyState : '' };
    } catch {
      return null;
    }
  };

  /**
   * 等待页面导航稳定：readyState 到 interactive/complete 且 href 连续多次一致。
   * 连续多次一致是为了避免"导航提交前误判旧页面已稳定"的竞态。
   * 返回 null 表示超时仍未稳定。
   */
  const waitForPageSettled = async (
    timeoutMs: number,
  ): Promise<{ href: string; readyState: string; loadFailed: boolean } | null> => {
    const deadline = Date.now() + timeoutMs;
    let lastHref: string | null = null;
    let stableCount = 0;
    while (Date.now() < deadline) {
      const state = await probePageState();
      if (state) {
        stableCount = state.href === lastHref ? stableCount + 1 : 1;
        lastHref = state.href;
        const ready = state.readyState === 'complete' || state.readyState === 'interactive';
        if (ready && stableCount >= SETTLE_STABLE_POLLS) {
          return { ...state, loadFailed: isChromeErrorUrl(state.href) };
        }
      }
      await wait(SETTLE_STEP_MS);
    }
    return null;
  };

  const runBrowserAutomationCommand = async (command: BrowserAutomationCommand) => {
    const input = automationInput(command);

    if (command.action === 'snapshot') {
      if (options.currentUrl.value && !options.isBrowserWindowReady.value) {
        await options.ensureBrowserWindowReady();
        await wait(1000);
      }
      if (!options.currentUrl.value || !options.isBrowserWindowReady.value) {
        return browserStatePayload({
          contentAvailable: false,
          note: 'Browser window has no ready page to snapshot yet.',
        });
      }

      try {
        // 导航刚发生时文档可能还没解析完：先等 isLoading 复位，再对"假空页"重试。
        const loadDeadline = Date.now() + SNAPSHOT_LOAD_WAIT_MS;
        while (options.isLoading.value && Date.now() < loadDeadline) {
          await wait(SNAPSHOT_LOAD_STEP_MS);
        }

        let page: Record<string, unknown> | null = null;
        for (let attempt = 0; attempt < SNAPSHOT_EMPTY_RETRIES; attempt += 1) {
          page = await options.captureBrowserSnapshot();
          const url = typeof page.url === 'string' ? page.url : '';
          if (isChromeErrorUrl(url)) break;
          const text = typeof page.text === 'string' ? page.text.trim() : '';
          const elements = Array.isArray(page.elements) ? page.elements : [];
          if (text || elements.length > 0) break;
          if (attempt < SNAPSHOT_EMPTY_RETRIES - 1) {
            await wait(SNAPSHOT_RETRY_STEP_MS);
          }
        }

        const pageUrl = typeof page?.url === 'string' ? page.url : '';
        const loadFailed = isChromeErrorUrl(pageUrl);
        const realUrl = pageUrl && !loadFailed ? pageUrl : null;
        if (realUrl) {
          syncUrlState(realUrl);
        }

        const frames = Array.isArray(page?.frames) ? page.frames : [];
        return browserStatePayload({
          contentAvailable: true,
          title: page?.title ?? null,
          text: page?.text ?? '',
          elements: page?.elements ?? [],
          headings: page?.headings ?? [],
          frames: loadFailed
            ? frames.map((frame: Record<string, unknown>, index: number) =>
                index === 0 && !frame.error
                  ? { ...frame, error: 'Page failed to load (browser error page).' }
                  : frame,
              )
            : frames,
          ...(loadFailed
            ? {
                url: pageUrl,
                loadFailed: true,
                loadError:
                  'The page failed to load — the browser is showing an error page. Check the URL or network connectivity.',
                note: 'Snapshot captured a browser error page (navigation failed).',
              }
            : {
                note:
                  'Snapshot includes DOM text and visible interactive elements from the current Nova Browser window, including reachable iframes.',
              }),
        });
      } catch (error) {
        return browserStatePayload({
          contentAvailable: false,
          extractionError: error instanceof Error ? error.message : String(error),
          note: 'Browser state is available, but DOM snapshot extraction failed.',
        });
      }
    }

    if (command.action === 'navigate') {
      const url = typeof input.url === 'string' ? input.url.trim() : '';
      if (!url) {
        throw new Error("Missing 'url' argument");
      }
      options.visit(url);
      // 等待导航稳定并读取页面真实状态，而不是固定等 1 秒后照抄请求地址。
      const settled = await waitForPageSettled(NAVIGATE_SETTLE_TIMEOUT_MS);
      if (settled?.loadFailed) {
        // 真实页面是 chrome-error:// 错误页：必须报错，否则 Agent 会误以为打开成功。
        throw new Error(
          `Navigation failed: ${url} could not be loaded (the browser is showing an error page). Check the URL or network connectivity.`,
        );
      }
      if (!settled) {
        // 超时仍未稳定：如实报告加载中，让 Agent 决定等待还是重试。
        return browserStatePayload({
          action: 'navigate',
          isLoading: true,
          note: 'Navigation is still in progress; run nova_browser_snapshot later to confirm the page state.',
        });
      }
      syncUrlState(settled.href);
      return browserStatePayload({ action: 'navigate' });
    }

    if (command.action === 'click') {
      if (!options.currentUrl.value) {
        throw new Error('Browser has no page loaded');
      }
      if (!options.isBrowserWindowReady.value) {
        await options.ensureBrowserWindowReady();
        await wait(1000);
      }
      if (!options.isBrowserWindowReady.value) {
        throw new Error('Browser window is not ready for click');
      }
      const ref = typeof input.ref === 'string' ? input.ref.trim() : '';
      const hasRef = Boolean(ref);
      const hasSelector = typeof input.selector === 'string' && input.selector.trim().length > 0;
      const hasCoordinates = typeof input.x === 'number' && typeof input.y === 'number';
      if (!hasRef && !hasSelector && !hasCoordinates) {
        throw new Error("Provide 'ref' from nova_browser_snapshot, 'selector', or both 'x' and 'y'");
      }
      if (hasRef) {
        await options.clickSnapshotRef(ref);
      } else if (options.evalBrowserScriptResult) {
        const res = await options.evalBrowserScriptResult(clickScript(input));
        if (res && typeof res === 'object' && res.ok === false) {
          throw new Error(res.error || 'No browser element matched the click target.');
        }
      } else {
        await options.evalBrowserScript(clickScript(input));
      }
      // 等待点击触发的导航稳定（未导航的点击 href 不变，会很快收敛），
      // 返回操作后的真实 URL，而不是停留在点击前的旧地址。
      const settled = await waitForPageSettled(CLICK_SETTLE_TIMEOUT_MS);
      if (settled && !settled.loadFailed) {
        syncUrlState(settled.href);
      }
      return browserStatePayload({
        action: 'click',
        ...(settled
          ? {
              url: settled.href,
              loadFailed: settled.loadFailed,
              ...(settled.loadFailed
                ? {
                    note: 'The click navigated, but the target page failed to load (browser error page).',
                  }
                : {}),
            }
          : {
              isLoading: true,
              note: 'Click dispatched; the page is still settling. Run nova_browser_snapshot to confirm the result.',
            }),
      });
    }

    if (command.action === 'type') {
      if (!options.currentUrl.value) {
        throw new Error('Browser has no page loaded');
      }
      if (!options.isBrowserWindowReady.value) {
        await options.ensureBrowserWindowReady();
        await wait(1000);
      }
      if (!options.isBrowserWindowReady.value) {
        throw new Error('Browser window is not ready for typing');
      }
      if (typeof input.text !== 'string') {
        throw new Error("Missing 'text' argument");
      }
      const ref = typeof input.ref === 'string' ? input.ref.trim() : '';
      if (ref) {
        await options.typeSnapshotRef(ref, input);
      } else if (options.evalBrowserScriptResult) {
        const res = await options.evalBrowserScriptResult(typeScript(input));
        if (res && typeof res === 'object' && res.ok === false) {
          throw new Error(res.error || 'No editable browser element matched the typing target.');
        }
      } else {
        await options.evalBrowserScript(typeScript(input));
      }
      await wait(250);
      return browserStatePayload({ action: 'type' });
    }

    if (command.action === 'reset') {
      const clearData = input.clear_data === true || input.clearData === true;
      if (clearData && options.isBrowserWindowReady.value) {
        await options.clearBrowsingData().catch((error) => {
          console.warn('Browser clear data during reset failed:', error);
        });
      }
      options.setElementPickerActive(false);
      options.currentUrl.value = '';
      options.addressInput.value = '';
      options.history.value = [];
      options.historyIndex.value = -1;
      await clearBrowserTabState(options.rawConversationId());
      await options.closeNativeBrowserWindow();
      await options.updateBrowserSessionUrl();
      return browserStatePayload({ action: 'reset', clearedData: clearData });
    }

    throw new Error(`Unsupported browser action: ${command.action}`);
  };

  const handleBrowserAutomationCommand = async (event: { payload: BrowserAutomationCommand }) => {
    const command = event.payload;
    if (!command || command.conversationId !== options.conversationId()) {
      return;
    }

    try {
      const result = await runBrowserAutomationCommand(command);
      await reportBrowserCommandResult(command.requestId, true, result);
    } catch (error) {
      await reportBrowserCommandResult(
        command.requestId,
        false,
        browserStatePayload(),
        error instanceof Error ? error.message : String(error),
      );
    }
  };

  const listenBrowserAutomationCommands = async (): Promise<UnlistenFn> =>
    listen<BrowserAutomationCommand>('nova-browser-command', handleBrowserAutomationCommand);

  return {
    browserStatePayload,
    listenBrowserAutomationCommands,
    runBrowserAutomationCommand,
  };
}
