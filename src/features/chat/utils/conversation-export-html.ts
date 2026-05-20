import type { ChatAttachment, PersistedMessage } from "@/lib/chat-types";
import { renderMarkdown } from "@/lib/markdown-render";

type ConversationExportHtmlInput = {
  conversationId: string;
  title: string;
  exportedAt: string;
  messages: PersistedMessage[];
};

const escapeHtml = (value: string) =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");

const roleLabel = (role: string) => {
  if (role === "user") return "你";
  if (role === "assistant") return "Nova";
  return role || "message";
};

const attachmentLabel = (attachment: ChatAttachment) => {
  const kind = attachment.kind ? ` · ${attachment.kind}` : "";
  return `${attachment.sourceName}${kind}`;
};

const renderAttachments = (attachments?: ChatAttachment[]) => {
  if (!attachments?.length) return "";
  const items = attachments
    .map((attachment) => `<span class="attachment">${escapeHtml(attachmentLabel(attachment))}</span>`)
    .join("");
  return `<div class="attachments">${items}</div>`;
};

const renderMessage = (message: PersistedMessage, index: number) => {
  const role = message.role === "user" ? "user" : "assistant";
  const body = message.content?.trim()
    ? renderMarkdown(message.content)
    : '<p class="empty-message">（空消息）</p>';
  const reasoning = message.reasoning?.trim()
    ? `<details class="reasoning"><summary>AI 思考过程</summary><div class="md-body">${renderMarkdown(message.reasoning)}</div></details>`
    : "";

  return `
    <article class="message ${role}">
      <div class="message-meta">
        <span class="message-index">#${index + 1}</span>
        <span class="message-role">${roleLabel(message.role)}</span>
      </div>
      <div class="message-card">
        <div class="md-body">${body}</div>
        ${reasoning}
        ${renderAttachments(message.attachments)}
      </div>
    </article>
  `;
};

export function buildConversationExportHtml(input: ConversationExportHtmlInput): string {
  const messagesHtml = input.messages.map(renderMessage).join("\n");

  return `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapeHtml(input.title || "Nova 会话导出")}</title>
  <style>
    @page { margin: 16mm 14mm; }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      color: #1f1d18;
      background: #fbfaf7;
      font-family: "Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", "Segoe UI", sans-serif;
      font-size: 14px;
      line-height: 1.72;
    }
    .page {
      max-width: 860px;
      margin: 0 auto;
      padding: 28px 24px 40px;
    }
    .cover {
      margin-bottom: 24px;
      padding-bottom: 18px;
      border-bottom: 1px solid #e5dfd2;
    }
    .cover h1 {
      margin: 0 0 10px;
      font-size: 24px;
      line-height: 1.25;
      letter-spacing: -0.02em;
    }
    .cover-meta {
      color: #8a8172;
      font-size: 12px;
      word-break: break-all;
    }
    .message {
      break-inside: avoid;
      margin: 0 0 18px;
    }
    .message-meta {
      display: flex;
      gap: 8px;
      align-items: center;
      margin-bottom: 6px;
      color: #8a8172;
      font-size: 12px;
    }
    .message-index {
      font-family: "Cascadia Code", "SF Mono", monospace;
      color: #b8ac99;
    }
    .message-role {
      font-weight: 700;
      color: #6f6659;
    }
    .message-card {
      border: 1px solid #e7dfd1;
      border-radius: 14px;
      background: #fffdf8;
      padding: 14px 16px;
      box-shadow: 0 8px 24px rgba(44, 36, 24, 0.05);
    }
    .message.user .message-card {
      background: #f4f0e8;
      border-color: #ded5c4;
    }
    .md-body {
      font-size: 14px;
      line-height: 1.75;
      color: inherit;
      word-break: break-word;
    }
    .md-body > :first-child { margin-top: 0 !important; }
    .md-body > :last-child { margin-bottom: 0 !important; }
    .md-body h1, .md-body h2, .md-body h3, .md-body h4, .md-body h5, .md-body h6 {
      font-weight: 800;
      line-height: 1.3;
      margin: 1.15em 0 0.45em;
      color: #1a1a18;
      break-after: avoid;
    }
    .md-body h1 { font-size: 1.55em; }
    .md-body h2 {
      font-size: 1.28em;
      border-bottom: 1px solid #ebe5da;
      padding-bottom: 0.25em;
    }
    .md-body h3 { font-size: 1.12em; }
    .md-body p { margin: 0.6em 0; }
    .md-body strong { font-weight: 800; color: #15130f; }
    .md-body em { font-style: italic; }
    .md-body code:not(.hljs) {
      font-family: "Cascadia Code", "SF Mono", Consolas, monospace;
      font-size: 0.86em;
      background: #f0ede7;
      color: #a33b2d;
      padding: 1px 5px;
      border-radius: 4px;
      border: 1px solid #e5e1d8;
    }
    .hljs-block {
      margin: 0.85em 0;
      border-radius: 10px;
      overflow: hidden;
      border: 1px solid #ded7ca;
      background: #202020;
      break-inside: avoid;
    }
    .hljs-header {
      display: flex;
      justify-content: space-between;
      padding: 6px 12px;
      background: #2a2a2a;
      border-bottom: 1px solid #3a3a3a;
    }
    .hljs-lang {
      font-size: 11px;
      color: #bcbcbc;
      font-family: "Cascadia Code", monospace;
    }
    .hljs-copy { display: none; }
    .hljs-block code {
      display: block;
      padding: 12px 14px;
      color: #f2f2f2;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      font-size: 12px;
      line-height: 1.58;
      font-family: "Cascadia Code", "SF Mono", Consolas, monospace;
    }
    .md-body blockquote {
      margin: 0.85em 0;
      padding: 9px 14px;
      border-left: 3px solid #c8c0b2;
      background: #f8f5ef;
      border-radius: 0 8px 8px 0;
      color: #62594b;
    }
    .md-body ul, .md-body ol {
      margin: 0.55em 0;
      padding-left: 1.55em;
    }
    .md-body li { margin: 0.22em 0; }
    .md-body table {
      border-collapse: collapse;
      margin: 0.85em 0;
      font-size: 12px;
      max-width: 100%;
      overflow-wrap: anywhere;
    }
    .md-body th {
      background: #f4f0e8;
      font-weight: 700;
      color: #2a2820;
    }
    .md-body th, .md-body td {
      padding: 7px 10px;
      border: 1px solid #e5dfd2;
      vertical-align: top;
    }
    .md-body tr:nth-child(even) td { background: #fbf8f2; }
    .md-body hr {
      border: none;
      border-top: 1px solid #e5dfd2;
      margin: 1.2em 0;
    }
    .md-body a {
      color: #2a6496;
      text-decoration: none;
      overflow-wrap: anywhere;
    }
    .md-body img {
      max-width: 100%;
      border-radius: 8px;
    }
    .md-body details,
    .reasoning {
      margin: 0.85em 0;
      border: 1px solid #e5dfd2;
      border-radius: 8px;
      overflow: hidden;
    }
    .md-body summary,
    .reasoning summary {
      padding: 7px 12px;
      font-weight: 700;
      background: #f7f3ec;
    }
    .details-body,
    .reasoning .md-body {
      padding: 8px 12px;
    }
    .attachments {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      margin-top: 10px;
    }
    .attachment {
      display: inline-flex;
      border: 1px solid #e2d9c8;
      background: #f8f4ed;
      color: #716657;
      border-radius: 999px;
      padding: 2px 8px;
      font-size: 11px;
    }
    .empty-message { color: #9b9387; }
    @media print {
      body { background: #fff; }
      .page { padding: 0; }
      .message-card { box-shadow: none; }
      a { color: #2a6496; }
    }
  </style>
</head>
<body>
  <main class="page">
    <section class="cover">
      <h1>${escapeHtml(input.title || "Nova 会话导出")}</h1>
      <div class="cover-meta">Conversation ID: ${escapeHtml(input.conversationId)}</div>
      <div class="cover-meta">Exported at: ${escapeHtml(input.exportedAt)}</div>
    </section>
    ${messagesHtml}
  </main>
</body>
</html>`;
}
