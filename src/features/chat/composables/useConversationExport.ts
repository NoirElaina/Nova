import { ref, type Ref, type ComputedRef } from "vue";
import type { ConversationMeta } from "@/lib/chat-types";
import {
  exportConversation,
  exportRenderedConversationPdf,
  loadConversationHistory,
  type ConversationExportFormat,
} from "../services/chat-api";
import { buildConversationExportHtml } from "../utils/conversation-export-html";
import { emitToast } from "@/lib/toast";

export function useConversationExport(
  conversations: Ref<ConversationMeta[]> | ComputedRef<ConversationMeta[]>,
) {
  const exportingConversationId = ref<string | null>(null);
  const exportingFormat = ref<ConversationExportFormat | null>(null);

  const formatExportLabel = (format: ConversationExportFormat) => format.toUpperCase();

  const handleExportConversation = async (
    conversationId: string,
    format: ConversationExportFormat,
  ) => {
    if (exportingConversationId.value) {
      return;
    }

    exportingConversationId.value = conversationId;
    exportingFormat.value = format;

    try {
      const conversation = conversations.value.find((item) => item.id === conversationId);
      const title = conversation?.title || "New chat";
      const exportPath =
        format === "pdf"
          ? await exportRenderedConversationPdf(
              conversationId,
              title,
              buildConversationExportHtml({
                conversationId,
                title,
                exportedAt: new Date().toISOString(),
                messages: await loadConversationHistory(conversationId),
              }),
            )
          : await exportConversation(conversationId, "json");
      emitToast({
        variant: "success",
        source: "conversation-export",
        message: `${formatExportLabel(format)} 已导出到：${exportPath}`,
      });
    } catch (err) {
      console.error("Failed to export conversation:", err);
      emitToast({
        variant: "error",
        source: "conversation-export",
        message: `导出 ${formatExportLabel(format)} 失败。`,
      });
    } finally {
      exportingConversationId.value = null;
      exportingFormat.value = null;
    }
  };

  return {
    exportingConversationId,
    exportingFormat,
    handleExportConversation,
  };
}
