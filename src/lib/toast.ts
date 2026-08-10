import { listen } from '@tauri-apps/api/event';
import { getRawErrorText } from './error-display';

export const NOVA_TOAST_EVENT = 'nova-toast';
/** AI 主流程错误：在聊天界面以红色错误块展示原文，不进入消息数组。 */
export const NOVA_CHAT_ERROR_EVENT = 'nova-chat-error';

export type ToastVariant = 'error' | 'success' | 'info' | 'warning';

export type ToastPayload = {
  message: string;
  variant?: ToastVariant;
  source?: string;
};

export type ChatErrorPayload = {
  message: string;
  source?: string;
};

let handlersInstalled = false;
let backendErrorListenerInstalled = false;

export function emitChatError(payload: ChatErrorPayload): void {
  const message = payload.message?.trim();
  if (!message || typeof window === 'undefined') {
    return;
  }
  window.dispatchEvent(
    new CustomEvent<ChatErrorPayload>(NOVA_CHAT_ERROR_EVENT, {
      detail: { message, source: payload.source },
    }),
  );
}

export function emitToast(payload: ToastPayload): void {
  if (typeof window === 'undefined') {
    return;
  }
  window.dispatchEvent(new CustomEvent<ToastPayload>(NOVA_TOAST_EVENT, { detail: payload }));
}

export function emitErrorToast(action: string, err: unknown, source?: string): void {
  const raw = getRawErrorText(err);
  emitToast({
    message: raw ? `${action}：${raw}` : action,
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
    message?: string;
    stage?: string | null;
  }>('backend-error', (event) => {
    const payload = event.payload ?? {};
    const source = `${payload.source ?? ''}`.toLowerCase();
    const message = `${payload.message ?? ''}`.trim() || '后端处理失败，请稍后重试。';

    // 工具执行错误（含权限拒绝、参数校验失败、未知工具、MCP 工具失败、hook 拦截等）
    // 已经作为 tool_result 反馈给 AI，且聊天界面会展示工具调用状态与结果，
    // 再提示是重复打扰用户。后端日志与 stderr 仍会记录，仅前端不提示。
    if (source === 'tool.execute') {
      return;
    }

    // AI 主流程错误（模型请求、轮次执行等）：在对话框内以红色块展示错误原文，
    // 方便用户看到真实报错内容去排查；其余本地小操作错误仍走右上角 toast。
    if (source.startsWith('llm.')) {
      emitChatError({ source: 'backend-error', message });
      return;
    }

    emitToast({
      variant: 'error',
      source: 'backend-error',
      message,
    });
  });
}
