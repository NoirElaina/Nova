import { reactive, onMounted, onBeforeUnmount, type Ref } from 'vue';
import { emitToast } from '../lib/toast';
import { openBranch } from '../features/branch/branch-chat';

export interface UseSelectionPopoverOptions {
  chatAreaRef: Ref<HTMLElement | null>;
  conversationId: Ref<string | null | undefined>;
  onQuote: (text: string) => void;
}

export function useSelectionPopover(options: UseSelectionPopoverOptions) {
  const selectionPopover = reactive({
    visible: false,
    x: 0,
    y: 0,
    text: '',
  });

  const hideSelectionPopover = () => {
    selectionPopover.visible = false;
  };

  /** mouseup 后读取选区：仅当选区落在助手消息气泡内时弹出操作条。 */
  const captureTextSelection = () => {
    const container = options.chatAreaRef.value;
    const selection = window.getSelection();
    if (!container || !selection || selection.isCollapsed) {
      hideSelectionPopover();
      return;
    }
    const text = selection.toString().trim();
    if (!text) {
      hideSelectionPopover();
      return;
    }
    const anchorEl =
      selection.anchorNode instanceof Element
        ? selection.anchorNode
        : selection.anchorNode?.parentElement;
    const row = anchorEl?.closest?.('[data-role="assistant"][data-message-index]');
    if (!row || !container.contains(row)) {
      hideSelectionPopover();
      return;
    }
    if (selection.rangeCount === 0) {
      hideSelectionPopover();
      return;
    }
    const rect = selection.getRangeAt(0).getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) {
      hideSelectionPopover();
      return;
    }
    selectionPopover.x = Math.min(Math.max(rect.left + rect.width / 2, 90), window.innerWidth - 90);
    selectionPopover.y = Math.max(rect.top, 64);
    selectionPopover.text = text.length > 2000 ? text.slice(0, 2000) : text;
    selectionPopover.visible = true;
  };

  const handleChatMouseUp = () => {
    // 等浏览器完成选区最终化（双击选词、拖动选择都在 mouseup 后才稳定）
    window.setTimeout(captureTextSelection, 0);
  };

  /** 点击操作条以外区域时收起；若形成新选区，mouseup 会重新弹出。 */
  const handleDocumentMouseDown = (event: MouseEvent) => {
    if (!selectionPopover.visible) return;
    const target = event.target;
    if (target instanceof Element && target.closest('[data-selection-popover]')) {
      return;
    }
    hideSelectionPopover();
  };

  const handleSelectionEscape = (event: KeyboardEvent) => {
    if (event.key === 'Escape') {
      hideSelectionPopover();
    }
  };

  const handleQuoteToInput = () => {
    options.onQuote(selectionPopover.text);
    hideSelectionPopover();
    window.getSelection()?.removeAllRanges();
  };

  const handleOpenBranch = () => {
    const cid = options.conversationId.value;
    if (!cid) {
      emitToast({ message: '请先开始当前对话，再使用分支提问', variant: 'warning' });
      return;
    }
    openBranch(cid, selectionPopover.text);
    hideSelectionPopover();
    window.getSelection()?.removeAllRanges();
  };

  onMounted(() => {
    document.addEventListener('mousedown', handleDocumentMouseDown, true);
    document.addEventListener('keydown', handleSelectionEscape);
  });

  onBeforeUnmount(() => {
    document.removeEventListener('mousedown', handleDocumentMouseDown, true);
    document.removeEventListener('keydown', handleSelectionEscape);
  });

  return {
    selectionPopover,
    hideSelectionPopover,
    handleChatMouseUp,
    handleQuoteToInput,
    handleOpenBranch,
  };
}
