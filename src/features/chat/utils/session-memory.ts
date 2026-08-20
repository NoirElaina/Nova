export function buildConversationTitle(source: string): string {
  const t = source.trim();
  return t.length > 24 ? `${t.slice(0, 24)}...` : t;
}
