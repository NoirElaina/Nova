<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Webview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  clearBrowserTabState,
  getBrowserTabState,
  saveBrowserTabState,
} from '../../../features/browser/browser-tab-state';

const props = defineProps<{
  conversationId?: string | null;
  visible?: boolean;
}>();

const initialState = getBrowserTabState(props.conversationId);
const addressInput = ref(initialState?.addressInput ?? '');
const currentUrl = ref(initialState?.currentUrl ?? '');
const history = ref<string[]>(initialState?.history ?? []);
const historyIndex = ref(initialState?.historyIndex ?? -1);
const isLoading = ref(false);
const isMenuOpen = ref(false);
const zoomPercent = ref(initialState?.zoomPercent ?? 100);
const showDeviceToolbar = ref(initialState?.showDeviceToolbar ?? false);
const isElementPickerActive = ref(false);
const browserHost = ref<HTMLElement | null>(null);
const browserLabel = `nova-browser-${crypto.randomUUID()}`;
let browserWebview: Webview | null = null;
let isBrowserWebviewReady = false;
let resizeObserver: ResizeObserver | null = null;
let pickerRetryTimers: number[] = [];
let restoreRetryTimer: number | null = null;
let unlistenBrowserCommand: UnlistenFn | null = null;
type BrowserSnapshotTarget = {
  selector?: string;
  frameId?: string;
  contextId?: number;
};

let browserSnapshotRefMap = new Map<string, BrowserSnapshotTarget>();

type BrowserFrameInfo = {
  frameId: string;
  parentFrameId?: string;
  name?: string;
  url?: string;
  depth: number;
  index: number;
};

type BrowserFrameSnapshot = {
  frame: BrowserFrameInfo;
  page?: Record<string, any>;
  contextId?: number;
  error?: string;
};

type BrowserAutomationCommand = {
  conversationId: string;
  requestId: string;
  action: string;
  payload?: {
    input?: Record<string, unknown>;
  };
};

const canGoBack = computed(() => historyIndex.value > 0);
const canGoForward = computed(() => historyIndex.value >= 0 && historyIndex.value < history.value.length - 1);
const browserConversationId = computed(() => props.conversationId?.trim() || '__default__');

const persistBrowserTabState = (conversationId: string | null | undefined = props.conversationId) => {
  saveBrowserTabState(conversationId, {
    addressInput: addressInput.value,
    currentUrl: currentUrl.value,
    history: history.value,
    historyIndex: historyIndex.value,
    zoomPercent: zoomPercent.value,
    showDeviceToolbar: showDeviceToolbar.value,
  });
};

const restoreBrowserTabState = (conversationId: string | null | undefined) => {
  const state = getBrowserTabState(conversationId);
  addressInput.value = state?.addressInput ?? '';
  currentUrl.value = state?.currentUrl ?? '';
  history.value = state?.history ?? [];
  historyIndex.value = state?.historyIndex ?? -1;
  zoomPercent.value = state?.zoomPercent ?? 100;
  showDeviceToolbar.value = state?.showDeviceToolbar ?? false;
};

const registerCurrentBrowserSession = async () => {
  await invoke('register_browser_session', {
    conversationId: props.conversationId || null,
    label: browserLabel,
    currentUrl: currentUrl.value || null,
  }).catch((error) => {
    console.warn('Browser session registration failed:', error);
  });
};

const unregisterBrowserSession = async (conversationId: string | null | undefined = props.conversationId) => {
  await invoke('unregister_browser_session', {
    conversationId: conversationId || null,
    label: browserLabel,
  }).catch((error) => {
    console.warn('Browser session unregister failed:', error);
  });
};

const updateBrowserSessionUrl = async () => {
  await invoke('update_browser_session_url', {
    conversationId: props.conversationId || null,
    label: browserLabel,
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
  schedulePickerInjection();
};

const getHostBounds = () => {
  const host = browserHost.value;
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

const hideBrowserSurface = async () => {
  persistBrowserTabState();
  pickerRetryTimers.forEach((timer) => window.clearTimeout(timer));
  pickerRetryTimers = [];
  if (restoreRetryTimer !== null) {
    window.clearTimeout(restoreRetryTimer);
    restoreRetryTimer = null;
  }
  isElementPickerActive.value = false;
  await applyElementPickerState().catch((error) => {
    console.warn('Browser element picker cleanup failed:', error);
  });
  if (browserWebview && isBrowserWebviewReady) {
    await browserWebview.hide().catch((error) => {
      console.warn('Browser webview hide failed:', error);
    });
  }
};

const closeBrowserSurface = async () => {
  await hideBrowserSurface();
  await closeNativeWebview();
};

defineExpose({ hideBrowserSurface, closeBrowserSurface });

const reopenSavedBrowserSurface = async () => {
  if (props.visible === false) return;
  if (!currentUrl.value) return;
  isLoading.value = true;
  const opened = await navigateNativeWebview(currentUrl.value);
  if (!opened) {
    isLoading.value = false;
    queueReopenSavedBrowserSurface();
  }
};

const queueReopenSavedBrowserSurface = () => {
  if (props.visible === false) return;
  if (!currentUrl.value || browserWebview || restoreRetryTimer !== null) return;
  restoreRetryTimer = window.setTimeout(() => {
    restoreRetryTimer = null;
    void reopenSavedBrowserSurface();
  }, 120);
};

const createNativeWebview = async (url: string): Promise<boolean> => {
  await nextTick();
  const rect = await waitForHostBounds();
  if (!rect) return false;
  if (browserWebview && !isBrowserWebviewReady) return false;

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
  return true;
};

const navigateNativeWebview = async (url: string): Promise<boolean> => {
  await nextTick();
  if (!browserWebview) {
    return createNativeWebview(url);
  }
  if (!isBrowserWebviewReady) return false;

  await syncWebviewBounds();
  await invoke('browser_navigate_webview', { label: browserLabel, url }).catch((error) => {
    console.warn('Browser navigation failed:', error);
  });
  window.setTimeout(() => {
    isLoading.value = false;
    schedulePickerInjection();
  }, 900);
  return true;
};

const evalBrowserScript = async (script: string) => {
  if (!browserWebview || !isBrowserWebviewReady) return;
  await invoke('browser_eval_webview_script', { label: browserLabel, script }).catch((error) => {
    console.warn('Browser script evaluation failed:', error);
  });
};

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

const browserStatePayload = (extra: Record<string, unknown> = {}) => ({
  url: currentUrl.value || null,
  address: addressInput.value || null,
  canGoBack: canGoBack.value,
  canGoForward: canGoForward.value,
  isLoading: isLoading.value,
  zoomPercent: zoomPercent.value,
  note:
    'Nova Browser v1 can navigate/click/type/reset the visible built-in browser and return a bounded DOM summary for the current page.',
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

const automationInput = (command: BrowserAutomationCommand) =>
  command.payload?.input ?? {};

const clickScript = (input: Record<string, unknown>) => {
  const selector = typeof input.selector === 'string' ? input.selector.trim() : '';
  const x = typeof input.x === 'number' ? input.x : null;
  const y = typeof input.y === 'number' ? input.y : null;

  return `
(() => {
  const selector = ${JSON.stringify(selector || null)};
  const x = ${JSON.stringify(x)};
  const y = ${JSON.stringify(y)};
  const target = selector
    ? document.querySelector(selector)
    : (Number.isFinite(x) && Number.isFinite(y) ? document.elementFromPoint(x, y) : null);
  if (!target) return { ok: false, error: 'No browser element matched the click target.' };
  target.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  const rect = target.getBoundingClientRect();
  const clientX = Number.isFinite(x) ? x : rect.left + rect.width / 2;
  const clientY = Number.isFinite(y) ? y : rect.top + rect.height / 2;
  for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
    target.dispatchEvent(new MouseEvent(type, {
      bubbles: true,
      cancelable: true,
      view: window,
      clientX,
      clientY,
    }));
  }
  return { ok: true };
})();
`;
};

const typeScript = (input: Record<string, unknown>) => {
  const selector = typeof input.selector === 'string' ? input.selector.trim() : '';
  const text = typeof input.text === 'string' ? input.text : '';
  const x = typeof input.x === 'number' ? input.x : null;
  const y = typeof input.y === 'number' ? input.y : null;
  const clear = input.clear === true;
  const submit = input.submit === true;

  return `
(() => {
  const selector = ${JSON.stringify(selector || null)};
  const text = ${JSON.stringify(text)};
  const x = ${JSON.stringify(x)};
  const y = ${JSON.stringify(y)};
  const clear = ${clear ? 'true' : 'false'};
  const submit = ${submit ? 'true' : 'false'};
  let target = selector ? document.querySelector(selector) : null;
  if (!target && Number.isFinite(x) && Number.isFinite(y)) {
    target = document.elementFromPoint(x, y);
  }
  if (!target) {
    target = document.activeElement;
  }
  if (!target || target === document.body || target === document.documentElement) {
    return { ok: false, error: 'No editable browser element matched the typing target.' };
  }
  target.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  if (typeof target.focus === 'function') target.focus();

  const isEditable = target.isContentEditable;
  if (isEditable) {
    if (clear) target.textContent = '';
    document.execCommand('insertText', false, text);
  } else {
    const currentValue = clear ? '' : (target.value || '');
    const nextValue = currentValue + text;
    const prototype = target instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, 'value');
    if (descriptor && descriptor.set) {
      descriptor.set.call(target, nextValue);
    } else {
      target.value = nextValue;
    }
    target.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: text }));
    target.dispatchEvent(new Event('change', { bubbles: true }));
  }

  if (submit) {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }));
    target.dispatchEvent(new KeyboardEvent('keyup', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }));
    if (target.form && typeof target.form.requestSubmit === 'function') {
      target.form.requestSubmit();
    }
  }
  return { ok: true };
})();
`;
};

const parseWebviewScriptResult = (raw: string) => {
  const first = JSON.parse(raw);
  if (typeof first === 'string') {
    return JSON.parse(first);
  }
  return first;
};

const browserSnapshotExpression = (refPrefix: string, maxTextLength = 9000, maxElements = 90) => `
(() => {
  const refPrefix = ${JSON.stringify(refPrefix)};
  const maxTextLength = ${maxTextLength};
  const maxElements = ${maxElements};
  const importantSelector = [
    'a[href]',
    'button',
    'input',
    'textarea',
    'select',
    'summary',
    '[role]',
    '[aria-label]',
    '[title]',
    'img[alt]',
    'img[title]',
    'iframe[title]',
    'iframe[name]',
    '[contenteditable="true"]',
    'h1',
    'h2',
    'h3',
    'h4',
    'h5',
    'h6'
  ].join(',');

  const visible = (element) => {
    if (!(element instanceof Element)) return false;
    const rect = element.getBoundingClientRect();
    if (rect.width < 2 || rect.height < 2) return false;
    const style = window.getComputedStyle(element);
    return style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity || '1') > 0.01;
  };

  const cleanText = (value) => String(value || '').replace(/\\s+/g, ' ').trim();
  const roleFor = (element) => {
    const explicitRole = element.getAttribute('role');
    if (explicitRole) return explicitRole;
    const tag = element.tagName.toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'button') return 'button';
    if (tag === 'input') return element.getAttribute('type') || 'input';
    if (tag === 'img') return 'image';
    if (tag === 'iframe') return 'frame';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    return tag;
  };
  const labelFor = (element) => {
    const aria = cleanText(element.getAttribute('aria-label'));
    if (aria) return aria;
    const title = cleanText(element.getAttribute('title'));
    if (title) return title;
    const placeholder = cleanText(element.getAttribute('placeholder'));
    if (placeholder) return placeholder;
    const alt = cleanText(element.getAttribute('alt'));
    if (alt) return alt;
    const name = cleanText(element.getAttribute('name'));
    if (name) return name;
    const value = element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement
      ? cleanText(element.value)
      : '';
    if (value) return value;
    return cleanText(element.innerText || element.textContent).slice(0, 220);
  };

  const elements = [];
  const seen = new Set();
  for (const element of Array.from(document.querySelectorAll(importantSelector))) {
    if (elements.length >= maxElements) break;
    if (seen.has(element) || !visible(element)) continue;
    const name = labelFor(element);
    if (!name) continue;
    seen.add(element);
    const ref = refPrefix + (elements.length + 1);
    element.setAttribute('data-nova-browser-ref', ref);
    const rect = element.getBoundingClientRect();
    elements.push({
      ref,
      role: roleFor(element),
      name,
      selector: '[data-nova-browser-ref="' + ref + '"]',
      tag: element.tagName.toLowerCase(),
      x: Math.round(rect.left + rect.width / 2),
      y: Math.round(rect.top + rect.height / 2),
      href: element instanceof HTMLAnchorElement ? element.href : null,
      source: element instanceof HTMLImageElement ? (element.currentSrc || element.src || null) : null,
    });
  }

  const headings = Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6'))
    .filter(visible)
    .slice(0, 60)
    .map((element) => ({
      level: Number(element.tagName.slice(1)),
      text: cleanText(element.innerText || element.textContent).slice(0, 220),
    }))
    .filter((heading) => heading.text);

  const textChunks = [];
  let textLength = 0;
  let visitedTextNodes = 0;
  const blockedTags = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG']);
  const walkerRoot = document.body || document.documentElement;
  if (walkerRoot) {
    const walker = document.createTreeWalker(walkerRoot, NodeFilter.SHOW_TEXT, {
      acceptNode(node) {
        if (visitedTextNodes >= 900 || textLength >= maxTextLength) {
          return NodeFilter.FILTER_REJECT;
        }
        const text = cleanText(node.nodeValue);
        if (!text) return NodeFilter.FILTER_REJECT;
        const parent = node.parentElement;
        if (!parent || !visible(parent)) return NodeFilter.FILTER_REJECT;
        if (blockedTags.has(parent.tagName)) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    while (walker.nextNode() && visitedTextNodes < 900 && textLength < maxTextLength) {
      visitedTextNodes += 1;
      const text = cleanText(walker.currentNode.nodeValue);
      textChunks.push(text);
      textLength += text.length + 1;
    }
  }
  const bodyText = textChunks.join('\\n').slice(0, maxTextLength);

  return {
    mode: 'runtime-dom-summary',
    title: document.title || '',
    url: location.href,
    text: bodyText,
    elements,
    headings,
    language: document.documentElement ? document.documentElement.lang || '' : '',
    capturedAt: new Date().toISOString(),
  };
})()
`;

const callDevtools = async (method: string, params: Record<string, unknown>) => {
  const raw = await invoke<string>('browser_call_devtools_protocol_method', {
    label: browserLabel,
    method,
    paramsJson: JSON.stringify(params),
  });
  return parseWebviewScriptResult(raw) as Record<string, any>;
};

const flattenBrowserFrameTree = (frameTree: any, depth = 0, frames: BrowserFrameInfo[] = []) => {
  const frame = frameTree?.frame;
  if (frame?.id) {
    frames.push({
      frameId: String(frame.id),
      parentFrameId: typeof frame.parentId === 'string' ? frame.parentId : undefined,
      name: typeof frame.name === 'string' ? frame.name : undefined,
      url: typeof frame.url === 'string' ? frame.url : undefined,
      depth,
      index: frames.length + 1,
    });
  }
  const children = Array.isArray(frameTree?.childFrames) ? frameTree.childFrames : [];
  for (const child of children) {
    flattenBrowserFrameTree(child, depth + 1, frames);
  }
  return frames;
};

const createFrameExecutionContext = async (frameId: string) => {
  const result = await callDevtools('Page.createIsolatedWorld', {
    frameId,
    worldName: 'nova-browser-snapshot',
    grantUniveralAccess: true,
  });
  const contextId = result.executionContextId;
  return typeof contextId === 'number' ? contextId : undefined;
};

const runtimeErrorMessage = (evaluated: Record<string, any>, fallback: string) => {
  if (evaluated.exceptionDetails) {
    return evaluated.exceptionDetails.exception?.description || evaluated.exceptionDetails.text || fallback;
  }
  const value = evaluated.result?.value;
  if (value && typeof value === 'object' && value.ok === false) {
    return typeof value.error === 'string' ? value.error : fallback;
  }
  return null;
};

const assertRuntimeEvaluationOk = (evaluated: Record<string, any>, fallback: string) => {
  const error = runtimeErrorMessage(evaluated, fallback);
  if (error) throw new Error(error);
};

const evaluateScriptInSnapshotContext = async (
  target: BrowserSnapshotTarget,
  expression: string,
  fallbackError: string,
) => {
  const params: Record<string, unknown> = {
    expression,
    returnByValue: true,
    awaitPromise: true,
  };
  if (typeof target.contextId === 'number') {
    params.contextId = target.contextId;
  }
  const evaluated = await callDevtools('Runtime.evaluate', params);
  assertRuntimeEvaluationOk(evaluated, fallbackError);
  return evaluated.result?.value;
};

const captureFrameSnapshot = async (
  frame: BrowserFrameInfo,
  topFrameId: string | undefined,
): Promise<BrowserFrameSnapshot> => {
  const refPrefix = `f${frame.index}_el`;
  let contextId: number | undefined;
  try {
    contextId = await createFrameExecutionContext(frame.frameId);
  } catch (error) {
    if (frame.frameId !== topFrameId) {
      throw error;
    }
  }

  const params: Record<string, unknown> = {
    expression: browserSnapshotExpression(refPrefix),
    returnByValue: true,
    awaitPromise: false,
  };
  if (typeof contextId === 'number') {
    params.contextId = contextId;
  }
  const evaluated = await callDevtools('Runtime.evaluate', params);
  assertRuntimeEvaluationOk(evaluated, 'Browser DOM snapshot script failed');

  const page = (evaluated.result?.value ?? {}) as Record<string, any>;
  const frameUrl = page.url || frame.url || '';
  const elements = Array.isArray(page.elements) ? page.elements : [];
  page.elements = elements.map((element: Record<string, any>) => ({
    ...element,
    frameId: frame.frameId,
    frameIndex: frame.index,
    frameName: frame.name || null,
    frameUrl,
  }));
  page.headings = Array.isArray(page.headings)
    ? page.headings.map((heading: Record<string, any>) => ({
        ...heading,
        frameId: frame.frameId,
        frameIndex: frame.index,
        frameUrl,
      }))
    : [];

  return {
    frame: {
      ...frame,
      url: frameUrl,
    },
    page,
    contextId,
  };
};

const captureBrowserSnapshot = async () => {
  const frameTreeResult = await callDevtools('Page.getFrameTree', {});
  const allFrames = flattenBrowserFrameTree(frameTreeResult.frameTree);
  const maxFrames = 35;
  const frames = allFrames.slice(0, maxFrames);
  const topFrameId = frames[0]?.frameId;

  browserSnapshotRefMap = new Map();
  const frameSnapshots: BrowserFrameSnapshot[] = [];
  for (const frame of frames) {
    try {
      const snapshot = await captureFrameSnapshot(frame, topFrameId);
      frameSnapshots.push(snapshot);
      const elements = Array.isArray(snapshot.page?.elements) ? snapshot.page.elements : [];
      for (const element of elements) {
        if (typeof element.ref !== 'string' || typeof element.selector !== 'string') continue;
        browserSnapshotRefMap.set(element.ref, {
          selector: element.selector,
          frameId: snapshot.frame.frameId,
          contextId: snapshot.contextId,
        });
      }
    } catch (error) {
      frameSnapshots.push({
        frame,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  const capturedFrames = frameSnapshots.filter((snapshot) => snapshot.page);
  const allElements = capturedFrames.flatMap((snapshot) =>
    Array.isArray(snapshot.page?.elements) ? snapshot.page.elements : [],
  );
  const allHeadings = capturedFrames.flatMap((snapshot) =>
    Array.isArray(snapshot.page?.headings) ? snapshot.page.headings : [],
  );
  const textByFrame = capturedFrames
    .map((snapshot) => {
      const page = snapshot.page ?? {};
      const text = typeof page.text === 'string' ? page.text.trim() : '';
      if (!text) return '';
      const title = typeof page.title === 'string' && page.title ? ` ${page.title}` : '';
      const url = snapshot.frame.url ? ` ${snapshot.frame.url}` : '';
      return `[Frame ${snapshot.frame.index}${title}${url}]\n${text}`;
    })
    .filter(Boolean)
    .join('\n\n')
    .slice(0, 52000);
  const topPage = capturedFrames[0]?.page ?? {};

  return {
    mode: 'cdp-frame-dom-summary',
    title: topPage.title ?? '',
    url: currentUrl.value || topPage.url || '',
    text: textByFrame,
    elements: allElements,
    headings: allHeadings,
    frames: frameSnapshots.map((snapshot) => ({
      frameId: snapshot.frame.frameId,
      parentFrameId: snapshot.frame.parentFrameId ?? null,
      index: snapshot.frame.index,
      depth: snapshot.frame.depth,
      name: snapshot.frame.name ?? null,
      url: snapshot.frame.url ?? null,
      title: snapshot.page?.title ?? null,
      elementCount: Array.isArray(snapshot.page?.elements) ? snapshot.page.elements.length : 0,
      textPreview:
        typeof snapshot.page?.text === 'string' ? snapshot.page.text.slice(0, 600) : '',
      error: snapshot.error ?? null,
    })),
    frameCount: allFrames.length,
    capturedFrameCount: capturedFrames.length,
    skippedFrameCount: Math.max(0, allFrames.length - frames.length),
    elementCount: allElements.length,
    capturedAt: new Date().toISOString(),
  } as Record<string, unknown>;
};

const clickSnapshotRef = async (ref: string) => {
  const target = browserSnapshotRefMap.get(ref);
  if (!target) {
    throw new Error(`Unknown browser snapshot ref: ${ref}. Run nova_browser_snapshot again.`);
  }
  if (target.selector) {
    await evaluateScriptInSnapshotContext(
      target,
      clickScript({ selector: target.selector }),
      `Unable to click browser snapshot ref: ${ref}`,
    );
    return;
  }
  throw new Error(`Browser snapshot ref has no click target: ${ref}`);
};

const typeSnapshotRef = async (ref: string, input: Record<string, unknown>) => {
  const target = browserSnapshotRefMap.get(ref);
  if (!target) {
    throw new Error(`Unknown browser snapshot ref: ${ref}. Run nova_browser_snapshot again.`);
  }
  if (!target.selector) {
    throw new Error(`Browser snapshot ref has no typing target: ${ref}`);
  }
  await evaluateScriptInSnapshotContext(
    target,
    typeScript({ ...input, selector: target.selector }),
    `Unable to type into browser snapshot ref: ${ref}`,
  );
};

const runBrowserAutomationCommand = async (command: BrowserAutomationCommand) => {
  const input = automationInput(command);

  if (command.action === 'snapshot') {
    if (!currentUrl.value || !browserWebview || !isBrowserWebviewReady) {
      return browserStatePayload({
        contentAvailable: false,
        note: 'Browser has no ready page to snapshot yet.',
      });
    }

    try {
      const page = await captureBrowserSnapshot();
      return browserStatePayload({
        contentAvailable: true,
        title: page.title ?? null,
        text: page.text ?? '',
        elements: page.elements ?? [],
        headings: page.headings ?? [],
        frames: page.frames ?? [],
        page,
        note:
          'Snapshot includes DOM text and visible interactive elements from the current Nova built-in browser page, including reachable iframes.',
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
    visit(url);
    await wait(1000);
    return browserStatePayload({ action: 'navigate' });
  }

  if (command.action === 'click') {
    if (!currentUrl.value) {
      throw new Error('Browser has no page loaded');
    }
    const ref = typeof input.ref === 'string' ? input.ref.trim() : '';
    const hasRef = Boolean(ref);
    const hasSelector = typeof input.selector === 'string' && input.selector.trim().length > 0;
    const hasCoordinates = typeof input.x === 'number' && typeof input.y === 'number';
    if (!hasRef && !hasSelector && !hasCoordinates) {
      throw new Error("Provide 'ref' from nova_browser_snapshot, 'selector', or both 'x' and 'y'");
    }
    if (hasRef) {
      await clickSnapshotRef(ref);
    } else {
      await evalBrowserScript(clickScript(input));
    }
    await wait(250);
    return browserStatePayload({ action: 'click' });
  }

  if (command.action === 'type') {
    if (!currentUrl.value) {
      throw new Error('Browser has no page loaded');
    }
    if (typeof input.text !== 'string') {
      throw new Error("Missing 'text' argument");
    }
    const ref = typeof input.ref === 'string' ? input.ref.trim() : '';
    if (ref) {
      await typeSnapshotRef(ref, input);
    } else {
      await evalBrowserScript(typeScript(input));
    }
    await wait(250);
    return browserStatePayload({ action: 'type' });
  }

  if (command.action === 'reset') {
    const clearData = input.clear_data === true || input.clearData === true;
    if (clearData && isBrowserWebviewReady) {
      await browserWebview?.clearAllBrowsingData().catch((error) => {
        console.warn('Browser clear data during reset failed:', error);
      });
    }
    isElementPickerActive.value = false;
    currentUrl.value = '';
    addressInput.value = '';
    history.value = [];
    historyIndex.value = -1;
    clearBrowserTabState(props.conversationId);
    await closeNativeWebview();
    await updateBrowserSessionUrl();
    return browserStatePayload({ action: 'reset', clearedData: clearData });
  }

  throw new Error(`Unsupported browser action: ${command.action}`);
};

const handleBrowserAutomationCommand = async (event: { payload: BrowserAutomationCommand }) => {
  const command = event.payload;
  if (!command || command.conversationId !== browserConversationId.value) {
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
  if (!browserWebview || !isBrowserWebviewReady) return;
  await cleanupLegacyElementPickerOverlays();
  try {
    await setNativeInspectMode(isElementPickerActive.value);
  } catch (error) {
    console.warn('Browser native inspect mode failed:', error);
    isElementPickerActive.value = false;
  }
};

const schedulePickerInjection = () => {
  pickerRetryTimers.forEach((timer) => window.clearTimeout(timer));
  pickerRetryTimers = [];
  if (!isElementPickerActive.value) return;
  [0, 120, 350, 900, 1800].forEach((delay) => {
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

watch(
  () => props.visible,
  (visible) => {
    if (visible === false) {
      void hideBrowserSurface();
      return;
    }
    if (browserWebview) {
      void nextTick().then(syncWebviewBounds);
    } else {
      queueReopenSavedBrowserSurface();
    }
  },
);

watch(currentUrl, () => {
  persistBrowserTabState();
  void updateBrowserSessionUrl();
});

watch([addressInput, history, historyIndex, zoomPercent, showDeviceToolbar], () => {
  persistBrowserTabState();
});

watch(
  () => props.conversationId,
  (next, previous) => {
    if (previous !== undefined && previous !== next) {
      persistBrowserTabState(previous);
      void unregisterBrowserSession(previous);
      restoreBrowserTabState(next);
      void closeNativeWebview().then(reopenSavedBrowserSurface);
    }
    void registerCurrentBrowserSession();
  },
);

watch(browserHost, (host, oldHost) => {
  if (oldHost) {
    resizeObserver?.unobserve(oldHost);
  }
  if (host) {
    resizeObserver?.observe(host);
  }
  if (browserWebview) {
    void syncWebviewBounds();
  } else {
    queueReopenSavedBrowserSurface();
  }
});

onMounted(() => {
  void registerCurrentBrowserSession();
  void listen<BrowserAutomationCommand>('nova-browser-command', handleBrowserAutomationCommand).then((unlisten) => {
    unlistenBrowserCommand = unlisten;
  }).catch((error) => {
    console.warn('Browser automation listener failed:', error);
  });

  resizeObserver = new ResizeObserver(() => {
    if (browserWebview) {
      void syncWebviewBounds();
    } else {
      queueReopenSavedBrowserSurface();
    }
  });
  if (browserHost.value) {
    resizeObserver.observe(browserHost.value);
  }
  window.addEventListener('resize', handleWindowResize);
  void reopenSavedBrowserSurface();
});

onBeforeUnmount(() => {
  unlistenBrowserCommand?.();
  void unregisterBrowserSession();
  resizeObserver?.disconnect();
  window.removeEventListener('resize', handleWindowResize);
  void closeBrowserSurface();
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
