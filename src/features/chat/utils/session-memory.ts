import type { ChatMessage } from "../../../lib/chat-types";

export function buildConversationTitle(source: string): string {
  const t = source.trim();
  return t.length > 24 ? `${t.slice(0, 24)}...` : t;
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
