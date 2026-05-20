import { listen } from '@tauri-apps/api/event';
import { formatUserFacingError } from './error-display';
import { formatBackendErrorEvent } from './error-display';

export const NOVA_TOAST_EVENT = 'nova-toast';

export type ToastVariant = 'error' | 'success' | 'info' | 'warning';

export type ToastPayload = {
  message: string;
  variant?: ToastVariant;
  source?: string;
};

let handlersInstalled = false;
let backendErrorListenerInstalled = false;

export function emitToast(payload: ToastPayload): void {
  if (typeof window === 'undefined') {
    return;
  }
  window.dispatchEvent(new CustomEvent<ToastPayload>(NOVA_TOAST_EVENT, { detail: payload }));
}

export function emitErrorToast(action: string, err: unknown, source?: string): void {
  const detail = formatUserFacingError(err);
  emitToast({
    message: detail ? `${action}：${detail}` : action,
    variant: 'error',
    source,
  });
}

export function installGlobalErrorToastHandlers(): void {
  if (handlersInstalled || typeof window === 'undefined') {
    return;
  }
  handlersInstalled = true;

  window.addEventListener('error', (event) => {
    const detail = event.error ?? event.message ?? '未知运行时错误';
    emitErrorToast('前端运行时错误', detail, 'window.error');
  });

  window.addEventListener('unhandledrejection', (event) => {
    emitErrorToast('未处理的异步错误', event.reason, 'window.unhandledrejection');
  });
}

export async function installBackendErrorToastListener(): Promise<void> {
  if (backendErrorListenerInstalled || typeof window === 'undefined') {
    return;
  }

  backendErrorListenerInstalled = true;
  await listen<{
    source?: string;
    code?: string;
    title?: string;
    message?: string;
    stage?: string | null;
  }>('backend-error', (event) => {
    const payload = event.payload ?? {};
    const source = `${payload.source ?? ''}`.toLowerCase();
    const message = `${payload.message ?? ''}`.toLowerCase();
    if (source === 'tool.execute' && message.includes('cancelled')) {
      return;
    }
    emitToast({
      variant: 'error',
      source: 'backend-error',
      message: formatBackendErrorEvent(payload),
    });
  });
}
