import { invoke } from '@tauri-apps/api/core';

export type BrowserTabState = {
  addressInput: string;
  currentUrl: string;
  history: string[];
  historyIndex: number;
  zoomPercent: number;
  showDeviceToolbar: boolean;
  updatedAt: number;
};

export type BrowserTabStateInput = Omit<BrowserTabState, 'updatedAt'>;

const states = new Map<string, BrowserTabState>();

export const browserStateKey = (conversationId?: string | null) => conversationId?.trim() || '__default__';

export const getBrowserTabState = (conversationId?: string | null): BrowserTabState | null => {
  const state = states.get(browserStateKey(conversationId));
  if (!state) return null;
  return {
    ...state,
    history: [...state.history],
  };
};

export const saveBrowserTabState = (conversationId: string | null | undefined, state: BrowserTabStateInput) => {
  const nextState = {
    ...state,
    history: [...state.history],
    updatedAt: Date.now(),
  };
  states.set(browserStateKey(conversationId), nextState);
  void invoke('save_browser_tab_state', {
    conversationId: conversationId || null,
    state,
  }).catch((error) => {
    console.warn('Browser tab state persistence failed:', error);
  });
};

export const clearBrowserTabState = (conversationId?: string | null) => {
  states.delete(browserStateKey(conversationId));
  void invoke('clear_browser_tab_state', {
    conversationId: conversationId || null,
  }).catch((error) => {
    console.warn('Browser tab state clear failed:', error);
  });
};

export const loadBrowserTabState = async (
  conversationId?: string | null,
): Promise<BrowserTabState | null> => {
  const cached = getBrowserTabState(conversationId);
  if (cached) return cached;

  try {
    const state = await invoke<BrowserTabState | null>('load_browser_tab_state', {
      conversationId: conversationId || null,
    });
    if (!state) return null;
    states.set(browserStateKey(conversationId), {
      ...state,
      history: [...state.history],
    });
    return getBrowserTabState(conversationId);
  } catch (error) {
    console.warn('Browser tab state load failed:', error);
    return null;
  }
};
