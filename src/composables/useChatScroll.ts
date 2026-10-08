import { nextTick, ref, type ComputedRef, type Ref } from 'vue';
import type { ChatMessage } from '../lib/chat-types';

export interface UseChatScrollOptions {
  chatAreaRef: Ref<HTMLElement | null>;
  virtualRowsCount: ComputedRef<number>;
  rowVirtualizer: Ref<{
    scrollToIndex: (
      index: number,
      options?: { align?: 'start' | 'center' | 'end' | 'auto'; behavior?: 'auto' | 'smooth' },
    ) => void;
  }>;
  messages: ComputedRef<ChatMessage[]>;
  userTimelineItems: ComputedRef<Array<{ index: number; summary: string }>>;
  onScrolled?: () => void;
}

export function useChatScroll(options: UseChatScrollOptions) {
  const showScrollToBottom = ref(false);
  /** 用户是否贴近底部；流式增高时只在 true 时跟滚，避免抢滚动 */
  const stickToBottom = ref(true);
  const activeUserMessageIndex = ref<number | null>(null);

  const distanceFromBottomPx = () => {
    const el = options.chatAreaRef.value;
    if (!el) return 0;
    return el.scrollHeight - el.clientHeight - el.scrollTop;
  };

  const updateScrollToBottomVisibility = () => {
    if (!options.chatAreaRef.value) {
      showScrollToBottom.value = false;
      return;
    }
    const distance = distanceFromBottomPx();
    // 只有真正贴近底部才算"贴底"；阈值太大会在用户刚往上滚一点时
    // 被流式跟滚反复拉回底部，产生"拉扯好几下才能上去"的体感。
    stickToBottom.value = distance <= 40;
    showScrollToBottom.value = distance > 120;
  };

  /** 滚轮手势感知：用户主动上滑时立刻解除贴底跟滚，避免流式内容把视口拽回去 */
  const handleChatWheel = (event: WheelEvent) => {
    if (event.deltaY < 0) {
      // 只解除跟滚；此刻滚动尚未生效，不能立刻重算距离，否则又会被判回贴底
      stickToBottom.value = false;
    } else if (event.deltaY > 0) {
      // 下滑滚回底部附近时恢复跟滚
      updateScrollToBottomVisibility();
    }
  };

  /** 仅贴底时把视口钉在列表末尾；不调用 measure()，避免清空虚拟列表尺寸缓存导致狂抖 */
  const pinToBottomIfSticky = () => {
    if (!stickToBottom.value || !options.chatAreaRef.value) return;
    const count = options.virtualRowsCount.value;
    if (count <= 0) return;
    // 直接改 scrollTop 比 scrollToIndex 更稳：不触发额外 layout 估算抖动
    const el = options.chatAreaRef.value;
    el.scrollTop = el.scrollHeight;
    showScrollToBottom.value = false;
  };

  const scrollToBottom = async () => {
    await nextTick();
    stickToBottom.value = true;
    const count = options.virtualRowsCount.value;
    if (count > 0 && options.chatAreaRef.value) {
      options.chatAreaRef.value.scrollTop = options.chatAreaRef.value.scrollHeight;
      // 再对齐一次，等 virtualizer 用真实高度算完 totalSize
      requestAnimationFrame(() => {
        if (options.chatAreaRef.value) {
          options.chatAreaRef.value.scrollTop = options.chatAreaRef.value.scrollHeight;
        }
      });
    } else if (options.chatAreaRef.value) {
      options.chatAreaRef.value.scrollTop = options.chatAreaRef.value.scrollHeight;
    }
    updateScrollToBottomVisibility();
  };

  const scrollLastUserMessageToTop = async () => {
    await nextTick();
    let lastUser = -1;
    const msgs = options.messages.value;
    for (let i = msgs.length - 1; i >= 0; i -= 1) {
      if (msgs[i]?.role === 'user') {
        lastUser = i;
        break;
      }
    }
    if (lastUser >= 0) {
      options.rowVirtualizer.value.scrollToIndex(lastUser, { align: 'start' });
    } else {
      await scrollToBottom();
    }
  };

  const scrollLastUserMessageToBottom = async () => {
    await nextTick();
    let lastUser = -1;
    const msgs = options.messages.value;
    for (let i = msgs.length - 1; i >= 0; i -= 1) {
      if (msgs[i]?.role === 'user') {
        lastUser = i;
        break;
      }
    }
    if (lastUser >= 0) {
      options.rowVirtualizer.value.scrollToIndex(lastUser, { align: 'end' });
    } else {
      await scrollToBottom();
    }
    updateScrollToBottomVisibility();
  };

  const scrollLiveAssistantIntoView = async () => {
    await nextTick();
    // 新一轮生成：贴底跟滚，不要 align:start（表格/长文增高时会把视口顶来顶去）
    stickToBottom.value = true;
    pinToBottomIfSticky();
    updateScrollToBottomVisibility();
  };

  const updateActiveUserMessage = () => {
    const container = options.chatAreaRef.value;
    if (!container || options.userTimelineItems.value.length === 0) {
      activeUserMessageIndex.value = null;
      return;
    }

    const rows = Array.from(
      container.querySelectorAll<HTMLElement>('[data-role="user"][data-message-index]'),
    );
    if (rows.length === 0) {
      // 虚拟列表可能未挂载目标，按滚动比例估算
      const items = options.userTimelineItems.value;
      if (items.length === 0) {
        activeUserMessageIndex.value = null;
        return;
      }
      const maxScroll = Math.max(1, container.scrollHeight - container.clientHeight);
      const ratio = container.scrollTop / maxScroll;
      const approx = Math.min(items.length - 1, Math.floor(ratio * items.length));
      activeUserMessageIndex.value = items[approx]?.index ?? null;
      return;
    }

    const containerTop = container.getBoundingClientRect().top;
    let closestIndex: number | null = null;
    let closestDistance = Number.POSITIVE_INFINITY;

    for (const row of rows) {
      const rawIndex = row.dataset.messageIndex;
      if (!rawIndex) continue;
      const index = Number.parseInt(rawIndex, 10);
      if (!Number.isFinite(index)) continue;

      const distance = Math.abs(row.getBoundingClientRect().top - containerTop - 20);
      if (distance < closestDistance) {
        closestDistance = distance;
        closestIndex = index;
      }
    }

    activeUserMessageIndex.value = closestIndex;
  };

  const handleChatScroll = () => {
    updateScrollToBottomVisibility();
    updateActiveUserMessage();
    options.onScrolled?.();
  };

  const scrollToBottomSmooth = async () => {
    await nextTick();
    stickToBottom.value = true;
    if (options.chatAreaRef.value) {
      options.chatAreaRef.value.scrollTo({
        top: options.chatAreaRef.value.scrollHeight,
        behavior: 'smooth',
      });
    }
  };

  const scrollToMessageIndex = async (index: number) => {
    await nextTick();
    if (index < 0 || index >= options.messages.value.length) return;
    activeUserMessageIndex.value = index;
    options.rowVirtualizer.value.scrollToIndex(index, { align: 'start', behavior: 'smooth' });
  };

  return {
    showScrollToBottom,
    stickToBottom,
    activeUserMessageIndex,
    distanceFromBottomPx,
    updateScrollToBottomVisibility,
    handleChatWheel,
    pinToBottomIfSticky,
    scrollToBottom,
    scrollLastUserMessageToTop,
    scrollLastUserMessageToBottom,
    scrollLiveAssistantIntoView,
    updateActiveUserMessage,
    handleChatScroll,
    scrollToBottomSmooth,
    scrollToMessageIndex,
  };
}
