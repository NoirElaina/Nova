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
  states.set(browserStateKey(conversationId), {
    ...state,
    history: [...state.history],
    updatedAt: Date.now(),
  });
};

export const clearBrowserTabState = (conversationId?: string | null) => {
  states.delete(browserStateKey(conversationId));
};
