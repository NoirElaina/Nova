let messageIdSeq = 0;

/** 生成会话内稳定的前端消息 id（不依赖后端）。 */
export function createMessageId(prefix = "msg"): string {
  messageIdSeq += 1;
  return `${prefix}-${Date.now().toString(36)}-${messageIdSeq.toString(36)}`;
}

export function ensureMessageId<T extends { id?: string; role?: string; createdAt?: number }>(
  message: T,
  indexHint = 0,
): T & { id: string } {
  if (message.id && message.id.trim()) {
    return message as T & { id: string };
  }
  const role = message.role || "msg";
  const created = message.createdAt && message.createdAt > 0 ? message.createdAt : Date.now();
  return {
    ...message,
    id: `${role}-${created}-${indexHint}`,
  };
}
