import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "../../../lib/chat-types";

export function buildConversationTitle(source: string): string {
  const t = source.trim();
  return t.length > 24 ? `${t.slice(0, 24)}...` : t;
}

/** 同步粗估（兜底用，已有调用点保持兼容） */
export function estimateTokens(text: string): number {
  const n = text.trim().length;
  if (n <= 0) return 0;
  return Math.ceil(n / 4);
}

/** 异步精估：调后端 estimate_text_tokens，按 tokenizer 家族分流。 */
export async function estimateTokensAsync(text: string, protocol = "anthropic"): Promise<number> {
  if (!text.trim()) return 0;
  try {
    return await invoke<number>("estimate_text_tokens", { text, protocol });
  } catch {
    return estimateTokens(text);
  }
}

export function extractSessionMemory(messages: ChatMessage[]): { summary: string; keyFacts: string[] } {
  const recent = messages.slice(-12);
  const summaryParts = recent
    .slice(-6)
    .map((m) => `${m.role === "user" ? "用户" : "Nova"}: ${m.content.replace(/\s+/g, " ").slice(0, 120)}`);
  const summary = summaryParts.join(" | ").slice(0, 800);

  const facts: string[] = [];
  for (const m of recent) {
    const lines = m.content
      .split(/\n+/)
      .map((s) => s.trim())
      .filter(Boolean);
    for (const line of lines) {
      if (facts.length >= 8) break;
      if (line.length >= 12 && line.length <= 120) {
        facts.push(line);
      }
    }
    if (facts.length >= 8) break;
  }

  return { summary, keyFacts: facts };
}
