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
  UploadedRagFile,
} from "../../../lib/chat-types";
import type {
  ConversationTurnRuntimeState,
  ModelImageBlock,
  ModelMessage,
  ModelTextBlock,
} from "./chat-controller-types";

export function buildAssistantCost(
  currentInputTokens: number,
  currentOutputTokens: number,
  currentToolCalls: number,
  currentToolDurationMs: number,
  contextCompacts: ContextCompactSummary[],
  toolSummary?: ToolTurnSummary,
): TurnCost {
  return {
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

export function formatMessageContentForModel(msg: ChatMessage): string {
  const content = msg.content.trim();
  const names =
    msg.attachments
      ?.filter((item) => item.kind !== "image")
      .map((item) => item.sourceName)
      .filter(Boolean) ?? [];
  const ragNotice =
    names.length > 0
      ? `\n\n已上传文件（可在会话RAG中检索）：${names.join("，")}\n文件全文已在本轮上下文中直接提供，请直接使用。会话压缩后如需重新获取，可调用 rag_tool 工具检索。`
      : "";

  if (content) {
    return `${content}${ragNotice}`;
  }

  if (names.length > 0) {
    return `请优先结合我上传的文件回答。${ragNotice}`;
  }

  return "";
}

export function isDocumentUploadFile(
  file: PendingUploadFile,
): file is UploadedRagFile {
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

export function toAttachmentMeta(files: PendingUploadFile[]): ChatAttachment[] {
  return files.map((file) => {
    if (file.kind === "image") {
      return {
        sourceName: file.sourceName,
        mimeType: file.mimeType,
        size: file.size,
        kind: "image",
        mediaType: file.mediaType,
        data: file.data,
      };
    }

    return {
      sourceName: file.sourceName,
      mimeType: file.mimeType,
      size: file.size,
      kind: "document",
    };
  });
}

export function buildModelMessage(msg: ChatMessage): ModelMessage {
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
