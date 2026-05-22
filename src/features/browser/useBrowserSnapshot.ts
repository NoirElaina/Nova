import { invoke } from '@tauri-apps/api/core';
import type { Ref } from 'vue';

type BrowserSnapshotTarget = {
  selector?: string;
  frameId?: string;
  contextId?: number;
};

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

export const clickScript = (input: Record<string, unknown>) => {
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

export const typeScript = (input: Record<string, unknown>) => {
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

export function useBrowserSnapshot(browserLabel: string, currentUrl: Ref<string>) {
  let browserSnapshotRefMap = new Map<string, BrowserSnapshotTarget>();

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

  return {
    callDevtools,
    flattenBrowserFrameTree,
    createFrameExecutionContext,
    captureBrowserSnapshot,
    clickSnapshotRef,
    typeSnapshotRef,
  };
}
