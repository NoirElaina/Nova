export const BROWSER_ANNOTATION_SELECTED_EVENT = 'nova-browser-annotation-selected';

export type BrowserAnnotationSelection = {
  id: string;
  conversationId: string | null;
  pageUrl: string;
  pageTitle: string;
  selector: string;
  tagName: string;
  text: string;
  ariaLabel: string;
  href: string;
  role: string;
  rect: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  frameIndex?: number;
  frameUrl?: string;
  capturedAt: string;
};

export type BrowserAnnotationSelectedPayload = {
  conversationId: string | null;
  sourceName: string;
  content: string;
  selection: BrowserAnnotationSelection;
};

const truncate = (value: string, maxLength: number) => {
  const normalized = value.replace(/\s+/g, ' ').trim();
  if (normalized.length <= maxLength) return normalized;
  return `${normalized.slice(0, maxLength - 1)}…`;
};

const safeFileSegment = (value: string) =>
  truncate(value, 40)
    .replace(/[\\/:*?"<>|]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim() || 'element';

export const elementPickerCleanupScript = () => `
(() => {
  const cleanupKey = '__novaElementPickerCleanup';
  if (typeof window[cleanupKey] === 'function') {
    window[cleanupKey]({ immediate: true });
  }
  document.getElementById('nova-real-element-picker-style')?.remove();
  document.getElementById('nova-real-element-picker-hover')?.remove();
  document.getElementById('nova-real-element-picker-selected')?.remove();
  window.__novaElementPickerState = undefined;
  window[cleanupKey] = undefined;
})();
`;

export const elementPickerInstallScript = (options: {
  conversationId: string | null;
  pageUrl: string;
}) => `
(() => {
  const cleanupKey = '__novaElementPickerCleanup';
  const stateKey = '__novaElementPickerState';
  const styleId = 'nova-real-element-picker-style';
  const hoverId = 'nova-real-element-picker-hover';
  const selectedId = 'nova-real-element-picker-selected';
  const conversationId = ${JSON.stringify(options.conversationId)};
  const fallbackPageUrl = ${JSON.stringify(options.pageUrl)};

  if (typeof window[cleanupKey] === 'function') {
    window[cleanupKey]({ immediate: true });
  }

  const state = { selection: null };
  window[stateKey] = state;

  const style = document.createElement('style');
  style.id = styleId;
  style.textContent = \`
    .nova-real-element-picker-box {
      position: fixed;
      z-index: 2147483647;
      pointer-events: none;
      border: 2px solid #0a84ff;
      background: rgba(10, 132, 255, 0.16);
      box-shadow:
        0 0 0 1px rgba(255, 255, 255, 0.88),
        0 0 0 4px rgba(10, 132, 255, 0.14),
        0 10px 26px rgba(10, 132, 255, 0.18);
      transform-origin: center;
      transition:
        left 90ms ease,
        top 90ms ease,
        width 90ms ease,
        height 90ms ease,
        opacity 120ms ease,
        transform 120ms ease;
    }
    .nova-real-element-picker-box[hidden] {
      display: none !important;
    }
    .nova-real-element-picker-selected {
      animation: novaElementPickerSelect 560ms cubic-bezier(.16, 1, .3, 1);
      background: rgba(10, 132, 255, 0.22);
      box-shadow:
        0 0 0 1px rgba(255, 255, 255, 0.96),
        0 0 0 5px rgba(10, 132, 255, 0.22),
        0 16px 36px rgba(10, 132, 255, 0.24);
    }
    @keyframes novaElementPickerSelect {
      0% {
        opacity: 0;
        transform: scale(.96);
      }
      42% {
        opacity: 1;
        transform: scale(1.035);
      }
      100% {
        opacity: 1;
        transform: scale(1);
      }
    }
  \`;
  document.documentElement.appendChild(style);

  const createBox = (id) => {
    const box = document.createElement('div');
    box.id = id;
    box.className = 'nova-real-element-picker-box';
    box.hidden = true;
    document.documentElement.appendChild(box);
    return box;
  };

  const hoverBox = createBox(hoverId);
  const selectedBox = createBox(selectedId);
  const pickerNodes = new Set([style, hoverBox, selectedBox]);

  const trimText = (value, maxLength = 600) => {
    const text = String(value || '').replace(/\\s+/g, ' ').trim();
    return text.length > maxLength ? text.slice(0, maxLength - 1) + '…' : text;
  };

  const cssEscape = (value) => {
    if (window.CSS && typeof window.CSS.escape === 'function') {
      return window.CSS.escape(value);
    }
    return String(value).replace(/[^a-zA-Z0-9_-]/g, '\\\\$&');
  };

  const elementSelector = (element) => {
    if (!(element instanceof Element)) return '';
    if (element.id) return '#' + cssEscape(element.id);

    const parts = [];
    let current = element;
    while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 5) {
      const tag = current.tagName.toLowerCase();
      let part = tag;
      const classNames = Array.from(current.classList || [])
        .filter(Boolean)
        .slice(0, 2);
      if (classNames.length > 0) {
        part += '.' + classNames.map(cssEscape).join('.');
      }

      const parent = current.parentElement;
      if (parent) {
        const sameTagSiblings = Array.from(parent.children)
          .filter((child) => child.tagName === current.tagName);
        if (sameTagSiblings.length > 1) {
          part += ':nth-of-type(' + (sameTagSiblings.indexOf(current) + 1) + ')';
        }
      }

      parts.unshift(part);
      current = parent;
      if (current === document.body || current === document.documentElement) break;
    }
    return parts.join(' > ');
  };

  const isPickerNode = (node) => {
    if (!node || !(node instanceof Node)) return false;
    for (const pickerNode of pickerNodes) {
      if (node === pickerNode || pickerNode.contains(node)) return true;
    }
    return false;
  };

  const drawBox = (box, target) => {
    if (!(target instanceof Element) || isPickerNode(target)) {
      box.hidden = true;
      return false;
    }
    const rect = target.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      box.hidden = true;
      return false;
    }
    box.hidden = false;
    box.style.left = Math.max(0, rect.left) + 'px';
    box.style.top = Math.max(0, rect.top) + 'px';
    box.style.width = Math.max(1, rect.width) + 'px';
    box.style.height = Math.max(1, rect.height) + 'px';
    return true;
  };

  const cleanupListeners = () => {
    document.removeEventListener('mousemove', onMouseMove, true);
    document.removeEventListener('click', onClick, true);
    document.removeEventListener('keydown', onKeyDown, true);
  };

  const cleanup = (options = {}) => {
    cleanupListeners();
    hoverBox.remove();
    selectedBox.remove();
    style.remove();
    if (!options.keepState) {
      window[stateKey] = undefined;
    }
    window[cleanupKey] = undefined;
  };

  function onMouseMove(event) {
    const target = document.elementFromPoint(event.clientX, event.clientY);
    drawBox(hoverBox, target);
  }

  function onClick(event) {
    const target = document.elementFromPoint(event.clientX, event.clientY);
    if (!(target instanceof Element) || isPickerNode(target)) return;

    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();

    if (!drawBox(selectedBox, target)) return;
    selectedBox.classList.remove('nova-real-element-picker-selected');
    void selectedBox.offsetWidth;
    selectedBox.classList.add('nova-real-element-picker-selected');
    hoverBox.hidden = true;

    const rect = target.getBoundingClientRect();
    const link = target.closest('a[href]');
    state.selection = {
      id: 'browser-annotation-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 8),
      conversationId,
      pageUrl: window.location.href || fallbackPageUrl,
      pageTitle: document.title || '',
      selector: elementSelector(target),
      tagName: target.tagName.toLowerCase(),
      text: trimText(target.innerText || target.textContent || target.getAttribute('aria-label') || target.getAttribute('title') || ''),
      ariaLabel: trimText(target.getAttribute('aria-label') || ''),
      href: link ? String(link.href || '') : '',
      role: trimText(target.getAttribute('role') || ''),
      rect: {
        x: Math.round(rect.left),
        y: Math.round(rect.top),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      },
      capturedAt: new Date().toISOString(),
    };

    cleanupListeners();
    window.setTimeout(() => cleanup({ keepState: true }), 1600);
  }

  function onKeyDown(event) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    cleanup();
  }

  document.addEventListener('mousemove', onMouseMove, true);
  document.addEventListener('click', onClick, true);
  document.addEventListener('keydown', onKeyDown, true);
  window[cleanupKey] = cleanup;
})();
`;

export const elementPickerReadSelectionScript = () => `
(() => {
  const state = window.__novaElementPickerState;
  if (!state || !state.selection) return null;
  const selection = state.selection;
  state.selection = null;
  return selection;
})();
`;

export const buildBrowserAnnotationPayload = (
  selection: BrowserAnnotationSelection,
): BrowserAnnotationSelectedPayload => {
  const label = safeFileSegment(selection.text || selection.ariaLabel || selection.pageTitle || selection.tagName);
  const sourceName = `浏览器注释 - ${label}.md`;
  const lines = [
    '# 浏览器注释',
    '',
    `- 页面: ${selection.pageUrl || '未知'}`,
    selection.pageTitle ? `- 标题: ${selection.pageTitle}` : '',
    selection.frameUrl && selection.frameUrl !== selection.pageUrl ? `- Frame: ${selection.frameUrl}` : '',
    `- 元素: <${selection.tagName}>`,
    selection.role ? `- Role: ${selection.role}` : '',
    selection.selector ? `- CSS 选择器: \`${selection.selector}\`` : '',
    selection.href ? `- 链接: ${selection.href}` : '',
    `- 位置: x=${selection.rect.x}, y=${selection.rect.y}, w=${selection.rect.width}, h=${selection.rect.height}`,
    `- 采集时间: ${selection.capturedAt}`,
    '',
    '## 选中内容',
    '',
    selection.text || selection.ariaLabel || '(这个元素没有可读文本)',
  ].filter(Boolean);

  return {
    conversationId: selection.conversationId,
    sourceName,
    content: lines.join('\n'),
    selection,
  };
};
