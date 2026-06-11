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

export type BuildModelMessageOptions = {
  includeDocumentContent?: boolean;
};

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
  options: BuildModelMessageOptions = {},
): string {
  const includeDocumentContent = options.includeDocumentContent ?? true;
  const content = msg.content.trim();
  const documentAttachments =
    msg.attachments
      ?.filter((item) => item.kind === "document")
      .filter((item) => item.sourceName?.trim()) ?? [];

  const documentBlocks = documentAttachments
    .filter((item) => includeDocumentContent && item.content?.trim())
    .map((item) => {
      const meta = [
        `name=${item.sourceName}`,
        item.mimeType ? `mime=${item.mimeType}` : null,
        Number.isFinite(item.size) ? `size=${item.size}` : null,
        item.knowledgeStored ? "knowledge=stored" : "knowledge=inline",
      ]
        .filter(Boolean)
        .join(" ");
      return `<document ${meta}>\n${item.content?.trim()}\n</document>`;
    });

  const attachedDocumentContext =
    documentBlocks.length > 0
      ? `\n\n[Attached Documents]\nThese documents are attached directly for this turn. Use them as primary context for the user's request.\n${documentBlocks.join("\n\n")}`
      : "";

  const attachmentNotice =
    documentAttachments.length > 0 && documentBlocks.length === 0
      ? `\n\n[Attached Documents]\nDocument attachment metadata: ${documentAttachments.map((item) => {
          const status = item.knowledgeStored ? "stored in conversation knowledge base" : "attached to conversation";
          return `${item.sourceName} (${status})`;
        }).join(", ")}`
      : "";

  if (content) {
    return `${content}${attachedDocumentContext}${attachmentNotice}`;
  }

  if (documentBlocks.length > 0) {
    return `请优先结合我上传的文件回答。${attachedDocumentContext}`;
  }

  if (documentAttachments.length > 0) {
    return `请优先结合我上传的文件回答。${attachmentNotice}`;
  }

  return "";
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
  options: { includeDocumentContent?: boolean; includeImageData?: boolean } = {},
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
      content: options.includeDocumentContent ? file.content : undefined,
      knowledgeStored: file.knowledgeStored,
    };
  });
}

export function buildModelMessage(
  msg: ChatMessage,
  options: BuildModelMessageOptions = {},
): ModelMessage {
  const textContent = formatMessageContentForModel(msg, options);
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
