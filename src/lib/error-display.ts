function normalizeText(value: unknown): string {
  if (value instanceof Error) {
    return value.message;
  }
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/** 提取错误的原始文本（不做友好化改写），前端直接展示报错原文。 */
export function getRawErrorText(err: unknown): string {
  return normalizeText(err).trim();
}
