import { estimateTokens } from "../utils/session-memory";
import type {
  ChatAttachment,
  ChatMessage,
  ContextCompactSummary,
  ConversationMemory,
  PendingUploadFile,
  ToolTurnSummary,
  TurnCost,
  UploadedImageFile,
  UploadedDocumentFile,
} from "../../../lib/chat-types";
import type {
  ConversationTurnRuntimeState,
  ModelImageBlock,
  ModelMessage,
  ModelTextBlock,
} from "./chat-controller-types";

// 内联注入的纯文本文件：单文件上限与截断保留量（头尾各保留）。
const MAX_INLINE_DOC_CHARS = 50000;
const TRUNCATION_HEAD_TAIL_CHARS = 20000;

export function buildAssistantCost(
  currentInputTokens: number,
  currentOutputTokens: number,
  currentToolCalls: number,
  currentToolDurationMs: number,
  contextCompacts: ContextCompactSummary[],
  toolSummary?: ToolTurnSummary,
  previousCost?: TurnCost,
): TurnCost {
  return {
    cacheReadTokens: previousCost?.cacheReadTokens,
    cacheCreationTokens: previousCost?.cacheCreationTokens,
    billableInputTokens: previousCost?.billableInputTokens,
    inputCostUsd: previousCost?.inputCostUsd,
    outputCostUsd: previousCost?.outputCostUsd,
    cacheReadCostUsd: previousCost?.cacheReadCostUsd,
    cacheCreationCostUsd: previousCost?.cacheCreationCostUsd,
    totalCostUsd: previousCost?.totalCostUsd,
    pricingModel: previousCost?.pricingModel,
    inputTokens: currentInputTokens,
    outputTokens: currentOutputTokens,
    toolCalls: currentToolCalls,
    toolDurationMs: currentToolDurationMs,
    contextCompacts,
    toolSummary,
  };
}

export function buildAssistantCostForState(
  state: ConversationTurnRuntimeState,
  toolSummary?: ToolTurnSummary,
): TurnCost {
  return {
    cacheReadTokens: state.assistantTurnCost?.cacheReadTokens,
    cacheCreationTokens: state.assistantTurnCost?.cacheCreationTokens,
    billableInputTokens: state.assistantTurnCost?.billableInputTokens,
    inputCostUsd: state.assistantTurnCost?.inputCostUsd,
    outputCostUsd: state.assistantTurnCost?.outputCostUsd,
    cacheReadCostUsd: state.assistantTurnCost?.cacheReadCostUsd,
    cacheCreationCostUsd: state.assistantTurnCost?.cacheCreationCostUsd,
    totalCostUsd: state.assistantTurnCost?.totalCostUsd,
    pricingModel: state.assistantTurnCost?.pricingModel,
    inputTokens: state.currentInputTokens,
    outputTokens: state.currentOutputTokens,
    toolCalls: state.currentToolCalls,
    toolDurationMs: state.currentToolDurationMs,
    contextCompacts: state.currentContextCompacts,
    toolSummary,
  };
}

export function shouldPreservePendingPromptOnStop(
  turnState: string,
  stopReason: string,
): boolean {
  return (
    turnState === "awaiting_user_input" ||
    turnState === "needs_user_input" ||
    stopReason === "needs_user_input"
  );
}

export function estimateInputTokensForTurn(
  messages: ChatMessage[],
  conversationMemory: ConversationMemory | null,
  userText: string,
  attachmentNames: string[],
): number {
  const historyText = messages
    .slice(-12)
    .map((m) => m.content)
    .join("\n");
  const memoryText = conversationMemory
    ? `Summary: ${conversationMemory.summary}\nFacts: ${conversationMemory.keyFacts.join("; ")}`
    : "";
  const attachmentText = attachmentNames.length
    ? `Attachments: ${attachmentNames.join(", ")}`
    : "";
  return estimateTokens(`${historyText}\n${memoryText}\n${attachmentText}\n${userText}`);
}

export function formatMessageContentForModel(
  msg: ChatMessage,
): string {
  const text = msg.content.trim();

  const documentAttachments = (msg.attachments ?? [])
    .filter((item) => item.kind === "document" && item.content?.trim() && item.sourceName?.trim());

  if (documentAttachments.length === 0) {
    return text;
  }

  const blocks = documentAttachments.map((item) => {
    const content = item.content!.trim();
    const originalLength = content.length;
    let body: string;
    let notice = "";
    if (originalLength > MAX_INLINE_DOC_CHARS) {
      const head = content.slice(0, TRUNCATION_HEAD_TAIL_CHARS);
      const tail = content.slice(originalLength - TRUNCATION_HEAD_TAIL_CHARS);
      body = `${head}\n\n...[中间内容已截断]...\n\n${tail}`;
      notice = `\n[注意：内容很长（原始 ${originalLength} 字符），已截断为头尾各 ${TRUNCATION_HEAD_TAIL_CHARS} 字符，中间内容可能丢失。如需完整内容请告知用户。]\n`;
    } else {
      body = content;
    }
    const meta = [`filename="${item.sourceName}"`, item.mimeType ? `mime="${item.mimeType}"` : null]
      .filter(Boolean)
      .join(" ");
    return `<document ${meta}>${notice}\n${body}\n</document>`;
  });

  const attachedDocumentContext = `\n\n[Attached Documents]\n${blocks.join("\n\n")}`;

  return text ? `${text}${attachedDocumentContext}` : attachedDocumentContext;
}

export function isDocumentUploadFile(
  file: PendingUploadFile,
): file is UploadedDocumentFile {
  return file.kind === "document";
}

export function isImageUploadFile(
  file: PendingUploadFile,
): file is UploadedImageFile {
  return file.kind === "image";
}

export function isImageAttachment(
  item: ChatAttachment,
): item is ChatAttachment & {
  kind: "image";
  mediaType: string;
  data: string;
} {
  return item.kind === "image" && !!item.mediaType && !!item.data;
}

export function toAttachmentMeta(
  files: PendingUploadFile[],
  options: { includeImageData?: boolean } = {},
): ChatAttachment[] {
  return files.map((file) => {
    if (file.kind === "image") {
      return {
        sourceName: file.sourceName,
        mimeType: file.mimeType,
        size: file.size,
        kind: "image",
        mediaType: file.mediaType,
        data: options.includeImageData ? file.data : undefined,
      };
    }

    return {
      sourceName: file.sourceName,
      mimeType: file.mimeType,
      size: file.size,
      kind: "document",
      content: file.content ?? undefined,
    };
  });
}

export function buildModelMessage(
  msg: ChatMessage,
): ModelMessage {
  const textContent = formatMessageContentForModel(msg);
  if (msg.role !== "user") {
    return {
      role: msg.role,
      content: textContent,
    };
  }

  const imageAttachments = (msg.attachments ?? []).filter(isImageAttachment);
  if (imageAttachments.length === 0) {
    return {
      role: msg.role,
      content: textContent,
    };
  }

  const fallbackText = textContent || "请结合我上传的图片回答。";
  const blocks: Array<ModelTextBlock | ModelImageBlock> = [
    {
      type: "text",
      text: fallbackText,
    },
  ];

  for (const image of imageAttachments) {
    const mediaType = (image.mediaType || image.mimeType || "").trim().toLowerCase();
    const data = (image.data || "").trim();
    if (!mediaType || !data) {
      continue;
    }

    blocks.push({
      type: "image",
      source: {
        type: "base64",
        media_type: mediaType,
        data,
      },
    });
  }

  if (blocks.length <= 1) {
    return {
      role: msg.role,
      content: fallbackText,
    };
  }

  return {
    role: msg.role,
    content: blocks,
  };
}
