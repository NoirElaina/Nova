import { invoke } from "@tauri-apps/api/core";

/**
 * 强类型 Tauri IPC 统一网关
 * 统一出参、异常捕获、错误格式化与调试日志。
 */
export class IpcGateway {
  static async call<TRes>(
    command: string,
    payload?: Record<string, unknown>,
  ): Promise<TRes> {
    try {
      return await invoke<TRes>(command, payload);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`[IPC Gateway Error: ${command}]`, message);
      throw new Error(message);
    }
  }
}

export const safeInvoke = IpcGateway.call;
