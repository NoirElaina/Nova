import type {
  ChatAttachment,
  ChatMessage,
  PendingUploadFile,
  UploadedImageFile,
  UploadedDocumentFile,
} from "../../../lib/chat-types";

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

/**
 * Redundant consecutive assistant message merging (pass-through / safeguard),
 * as backend `projection.rs` already projects aggregated messages directly.
 */
export function sanitizeConsecutiveAssistantMessages(rawMessages: ChatMessage[]): ChatMessage[] {
  const result: ChatMessage[] = [];
  for (const msg of rawMessages) {
    const prev = result[result.length - 1];
    if (msg.role === "assistant" && prev && prev.role === "assistant") {
      const parts = [prev.content, msg.content].filter((c) => c && c.trim());
      prev.content = parts.join("\n\n");
      const reasoningParts = [prev.reasoning, msg.reasoning].filter((r) => r && r.trim());
      if (reasoningParts.length > 0) {
        prev.reasoning = reasoningParts.join("\n\n");
      }
      if (msg.tokenUsage) {
        prev.tokenUsage = (prev.tokenUsage ?? 0) + msg.tokenUsage;
      }
      if (msg.cost) {
        prev.cost = { ...prev.cost, ...msg.cost };
      }
      if (msg.transcriptSegments) {
        prev.transcriptSegments = [
          ...(prev.transcriptSegments ?? []),
          ...msg.transcriptSegments,
        ];
      }
    } else {
      result.push({ ...msg });
    }
  }
  return result;
}
