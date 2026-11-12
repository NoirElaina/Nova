import { invoke } from "@tauri-apps/api/core";

/** 生成客户端幂等键（同一逻辑写操作重试时复用，防重放双写）。 */
export function newIdempotencyKey(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** 统一前端 IPC 入口，便于契约扫描与后续横切治理。 */
export function ipcInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return args === undefined ? invoke<T>(command) : invoke<T>(command, args);
}
