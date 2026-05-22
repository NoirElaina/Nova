import { invoke } from '@tauri-apps/api/core';
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';

export type BrowserTabState = {
  addressInput: string;
  currentUrl: string;
  history: string[];
  historyIndex: number;
  zoomPercent: number;
  updatedAt: number;
};

export type BrowserTabStateInput = Omit<BrowserTabState, 'updatedAt'>;

const states = new Map<string, BrowserTabState>();
const BROWSER_TAB_STATE_CLEARED_EVENT = 'nova-browser-tab-state-cleared';

export type BrowserTabStateClearedPayload = {
  conversationId: string | null;
  key: string;
};

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

export const clearBrowserTabState = async (conversationId?: string | null) => {
  const key = browserStateKey(conversationId);
  states.delete(key);
  try {
    await invoke('clear_browser_tab_state', {
      conversationId: conversationId || null,
    });
  } catch (error) {
    console.warn('Browser tab state clear failed:', error);
  }
  await emit(BROWSER_TAB_STATE_CLEARED_EVENT, {
    conversationId: conversationId || null,
    key,
  }).catch((error) => {
    console.warn('Browser tab state clear event failed:', error);
  });
};

export const listenBrowserTabStateCleared = (
  handler: (payload: BrowserTabStateClearedPayload) => void,
): Promise<UnlistenFn> =>
  listen<BrowserTabStateClearedPayload>(BROWSER_TAB_STATE_CLEARED_EVENT, (event) => {
    states.delete(event.payload.key);
    handler(event.payload);
  });

export const loadBrowserTabState = async (
  conversationId?: string | null,
): Promise<BrowserTabState | null> => {
  try {
    const state = await invoke<BrowserTabState | null>('load_browser_tab_state', {
      conversationId: conversationId || null,
    });
    if (!state) {
      states.delete(browserStateKey(conversationId));
      return null;
    }
    states.set(browserStateKey(conversationId), {
      ...state,
      history: [...state.history],
    });
    return getBrowserTabState(conversationId);
  } catch (error) {
    console.warn('Browser tab state load failed:', error);
    return getBrowserTabState(conversationId);
  }
};
