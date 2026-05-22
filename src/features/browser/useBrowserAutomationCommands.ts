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
  closeNativeBrowserWindow: () => Promise<void>;
  clearBrowsingData: () => Promise<void>;
  updateBrowserSessionUrl: () => Promise<void>;
  setElementPickerActive: (value: boolean) => void;
  captureBrowserSnapshot: () => Promise<Record<string, unknown>>;
  clickSnapshotRef: (ref: string) => Promise<void>;
  typeSnapshotRef: (ref: string, input: Record<string, unknown>) => Promise<void>;
};

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

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
        const page = await options.captureBrowserSnapshot();
        return browserStatePayload({
          contentAvailable: true,
          title: page.title ?? null,
          text: page.text ?? '',
          elements: page.elements ?? [],
          headings: page.headings ?? [],
          frames: page.frames ?? [],
          page,
          note:
            'Snapshot includes DOM text and visible interactive elements from the current Nova Browser window, including reachable iframes.',
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
      await wait(1000);
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
      } else {
        await options.evalBrowserScript(clickScript(input));
      }
      await wait(250);
      return browserStatePayload({ action: 'click' });
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
