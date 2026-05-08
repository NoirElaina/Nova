export const NOVA_TOAST_EVENT = 'nova-toast';

export type ToastVariant = 'error' | 'success' | 'info' | 'warning';

export type ToastPayload = {
  message: string;
  variant?: ToastVariant;
  source?: string;
};

let handlersInstalled = false;

export function normalizeErrorMessage(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }
  if (typeof err === 'string') {
    return err;
  }
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

export function emitToast(payload: ToastPayload): void {
  if (typeof window === 'undefined') {
    return;
  }
  window.dispatchEvent(new CustomEvent<ToastPayload>(NOVA_TOAST_EVENT, { detail: payload }));
}

export function emitErrorToast(action: string, err: unknown, source?: string): void {
  const detail = normalizeErrorMessage(err);
  emitToast({
    message: detail ? `${action}: ${detail}` : action,
    variant: 'error',
    source,
  });
}

export function installGlobalErrorToastHandlers(): void {
  if (handlersInstalled || typeof window === 'undefined') {
    return;
  }
  handlersInstalled = true;

  const originalConsoleError = console.error.bind(console);
  console.error = (...args: unknown[]) => {
    originalConsoleError(...args);
    const message = args
      .map((arg) => normalizeErrorMessage(arg))
      .filter(Boolean)
      .join(' | ')
      .slice(0, 500);
    if (message) {
      emitToast({ message, variant: 'error', source: 'console.error' });
    }
  };

  window.addEventListener('error', (event) => {
    const detail = event.error ?? event.message ?? '未知运行时错误';
    emitErrorToast('前端运行时错误', detail, 'window.error');
  });

  window.addEventListener('unhandledrejection', (event) => {
    emitErrorToast('未处理的异步错误', event.reason, 'window.unhandledrejection');
  });
}
