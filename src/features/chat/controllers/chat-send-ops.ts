import type { Ref } from "vue";
import { emitToast } from "../../../lib/toast";
import {
  buildPendingQuestionReply,
  extractPermissionActionFromAnswers,
} from "../../../lib/chat-payloads";
import type {
  AgentMode,
  AskUserAnswerSubmission,
  ChatAttachment,
  ChatMessage,
  ContextCompactSummary,
  ContextUsage,
  PendingUploadFile,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";
import {
  cancelChatMessage,
  replaceConversationHistory,
  sendChatMessage,
  submitPermissionDecision,
  estimateTextTokens,
  getActiveModelRuntime,
  upsertConversationRagDocuments,
} from "../services/chat-api";
import type {
  ChatScreenHandle,
  ConversationTurnRuntimeState,
  LiveTurnStage,
} from "./chat-controller-types";
import type { ActiveRuntimeRefs } from "./chat-runtime-state";
import {
  ensureRuntimeState,
  normalizeConversationId,
  resetPendingPromptState,
  resetToolTrackingState,
  resetTurnRuntimeState,
} from "./chat-runtime-state";
import {
  buildModelMessage,
  type BuildModelMessageOptions,
  isDocumentUploadFile,
  isImageUploadFile,
  toAttachmentMeta,
} from "./chat-message-helpers";

const INLINE_CONTEXT_WINDOW_RATIO = 0.82;
const MIN_INLINE_CONTEXT_TOKENS = 8_000;

type PreparedContextDispatch = {
  messages: ChatMessage[];
  buildOptions: BuildModelMessageOptions;
  rewroteHistory: boolean;
};

type ConversationDocumentForRag = {
  sourceName: string;
  mimeType?: string;
  content: string;
};

function modelMessageTextForEstimate(message: ChatMessage, options: BuildModelMessageOptions) {
  const built = buildModelMessage(message, options);
  if (typeof built.content === "string") {
    return built.content;
  }

  return built.content
    .filter((block): block is { type: "text"; text: string } => block.type === "text")
    .map((block) => block.text)
    .join("\n");
}

function collectInlineConversationDocuments(messages: ChatMessage[]): ConversationDocumentForRag[] {
  const byKey = new Map<string, ConversationDocumentForRag>();
  for (const message of messages) {
    for (const attachment of message.attachments ?? []) {
      if (attachment.kind !== "document") {
        continue;
      }
      const content = attachment.content?.trim();
      if (!content) {
        continue;
      }
      const sourceName = attachment.sourceName?.trim();
      if (!sourceName) {
        continue;
      }
      byKey.set(`${sourceName}\n${content}`, {
        sourceName,
        mimeType: attachment.mimeType,
        content,
      });
    }
  }
  return [...byKey.values()];
}

function markConversationDocumentsStored(messages: ChatMessage[]): ChatMessage[] {
  return messages.map((message) => {
    if (!message.attachments?.some((attachment) => attachment.kind === "document")) {
      return message;
    }

    return {
      ...message,
      attachments: message.attachments.map((attachment) => {
        if (attachment.kind !== "document") {
          return attachment;
        }

        return {
          sourceName: attachment.sourceName,
          mimeType: attachment.mimeType,
          size: attachment.size,
          kind: "document" as const,
          knowledgeStored: true,
        };
      }),
    };
  });
}

type SendOpsDeps = {
  activeConversationId: Ref<string>;
  isGenerating: Ref<boolean>;
  currentStage: Ref<LiveTurnStage>;
  messages: Ref<ChatMessage[]>;
  toolExecutionLogs: Ref<ToolExecutionEntry[]>;
  pendingUploads: Ref<PendingUploadFile[]>;
  pendingPermissionRequestId: Ref<string | null>;
  mainView: Ref<"chat" | "custom" | "hooks" | "agent" | "agentMarket" | "schedule">;
  planMode: Ref<boolean>;
  agentMode: Ref<AgentMode>;
  assistantResponse: Ref<string>;
  assistantReasoning: Ref<string>;
  assistantTokenUsage: Ref<number | undefined>;
  assistantTurnCost: Ref<TurnCost | undefined>;
  currentToolStartedAt: Ref<number | null>;
  currentToolCalls: Ref<number>;
  currentToolDurationMs: Ref<number>;
  currentContextUsage: Ref<ContextUsage | undefined>;
  currentContextCompacts: Ref<ContextCompactSummary[]>;
  currentContextTokens: Ref<number>;
  currentInputTokens: Ref<number>;
  currentOutputTokens: Ref<number>;
  currentTurnId: Ref<string | null>;
  chatScreenRef: Ref<ChatScreenHandle | null>;
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>;
  activeRuntimeRefs: ActiveRuntimeRefs;
  createNewConversation: (seedTitle?: string) => Promise<string | null>;
  persistMessage: (message: ChatMessage, conversationId?: string) => Promise<void>;
  refreshConversationFiles: (conversationId: string) => Promise<void>;
  resetBackgroundRuntimeState: (
    conversationId: string,
    state: ConversationTurnRuntimeState,
    preservePendingPrompt?: boolean,
  ) => void;
};

export function createSendOperations(deps: SendOpsDeps) {
  const {
    activeConversationId,
    isGenerating,
    currentStage,
    messages,
    toolExecutionLogs,
    pendingUploads,
    pendingPermissionRequestId,
    mainView,
    planMode,
    agentMode,
    assistantResponse,
    assistantReasoning,
    assistantTokenUsage,
    assistantTurnCost,
    currentToolStartedAt,
    currentToolCalls,
    currentToolDurationMs,
    currentContextUsage,
    currentContextCompacts,
    currentContextTokens,
    currentInputTokens,
    currentOutputTokens,
    currentTurnId,
    chatScreenRef,
    runtimeStateByConversation,
    activeRuntimeRefs,
    createNewConversation,
    persistMessage,
    refreshConversationFiles,
    resetBackgroundRuntimeState,
  } = deps;

  async function prepareConversationContextForDispatch(
    sendingConversationId: string,
    nextMessages: ChatMessage[],
  ): Promise<PreparedContextDispatch> {
    const inlineDocuments = collectInlineConversationDocuments(nextMessages);
    if (inlineDocuments.length === 0) {
      return {
        messages: nextMessages,
        buildOptions: { includeDocumentContent: true },
        rewroteHistory: false,
      };
    }

    try {
      const runtime = await getActiveModelRuntime();
      const estimatedPromptText = nextMessages
        .map((message) => modelMessageTextForEstimate(message, { includeDocumentContent: true }))
        .join("\n\n");
      const estimatedTokens = await estimateTextTokens(estimatedPromptText, runtime.protocol);
      const inlineLimit = Math.max(
        MIN_INLINE_CONTEXT_TOKENS,
        Math.floor(runtime.windowTokens * INLINE_CONTEXT_WINDOW_RATIO),
      );

      if (estimatedTokens <= inlineLimit) {
        return {
          messages: nextMessages,
          buildOptions: { includeDocumentContent: true },
          rewroteHistory: false,
        };
      }

      const result = await upsertConversationRagDocuments(
        sendingConversationId,
        inlineDocuments.map((file) => ({
          sourceName: file.sourceName,
          sourceType: "file",
          mimeType: file.mimeType,
          content: file.content,
        })),
      );

      if (result.rejected.length > 0 || result.added + result.updated <= 0) {
        const detail = result.rejected
          .slice(0, 2)
          .map((item) => `${item.sourceName}(${item.reason})`)
          .join("；");
        emitToast({
          variant: "warning",
          source: "upload",
          message: detail
            ? `文档超过上下文窗口，但自动入库不完整：${detail}。本轮仍尝试按全文发送。`
            : "文档超过上下文窗口，但自动入库未完成。本轮仍尝试按全文发送。",
        });
        return {
          messages: nextMessages,
          buildOptions: { includeDocumentContent: true },
          rewroteHistory: false,
        };
      }

      await refreshConversationFiles(sendingConversationId);
      emitToast({
        variant: "info",
        source: "upload",
        message: "文档已超过当前模型上下文窗口，已自动转入会话知识库。",
      });

      return {
        messages: markConversationDocumentsStored(nextMessages),
        buildOptions: { includeDocumentContent: false },
        rewroteHistory: true,
      };
    } catch (err) {
      console.error("Failed to prepare conversation document context:", err);
      emitToast({
        variant: "warning",
        source: "upload",
        message: "文档上下文策略判断失败，本轮仍尝试按全文发送。",
      });
      return {
        messages: nextMessages,
        buildOptions: { includeDocumentContent: true },
        rewroteHistory: false,
      };
    }
  }

  async function dispatchConversationMessages(
    sendingConversationId: string,
    nextMessages: ChatMessage[],
    buildOptions: BuildModelMessageOptions = {},
  ) {
    if (activeConversationId.value !== sendingConversationId) {
      emitToast({
        variant: "info",
        source: "send",
        message: "会话已切换，本次发送已取消，请在当前会话重新发送。",
      });
      return;
    }

    isGenerating.value = true;
    currentStage.value = "processing";
    assistantResponse.value = "";
    assistantReasoning.value = "";
    assistantTokenUsage.value = undefined;
    assistantTurnCost.value = undefined;
    currentToolStartedAt.value = null;
    currentToolCalls.value = 0;
    currentToolDurationMs.value = 0;
    currentContextUsage.value = undefined;
    currentContextCompacts.value = [];
    currentContextTokens.value = 0;
    currentOutputTokens.value = 0;
    currentInputTokens.value = 0;
    resetToolTrackingState(activeRuntimeRefs);
    currentTurnId.value = `turn-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    void chatScreenRef.value?.scrollLiveAssistantIntoView();

    const rustMessages = nextMessages.map((message) => buildModelMessage(message, buildOptions));

    try {
      await sendChatMessage(
        sendingConversationId || null,
        rustMessages,
        planMode.value,
        agentMode.value,
      );
    } catch (err: unknown) {
      const isActiveFailedConversation = activeConversationId.value === sendingConversationId;
      if (isActiveFailedConversation && !isGenerating.value) {
        return;
      }

      console.error("Chat error:", err);
      if (isActiveFailedConversation) {
        emitToast({
          variant: "error",
          source: "send",
          message: "消息发送失败，请检查后端日志后重试。",
        });
        assistantResponse.value = "";
        assistantReasoning.value = "";
        assistantTokenUsage.value = undefined;
        assistantTurnCost.value = undefined;
        isGenerating.value = false;
        resetTurnRuntimeState(activeRuntimeRefs);
        runtimeStateByConversation.delete(normalizeConversationId(sendingConversationId));
      } else {
        const backgroundState = ensureRuntimeState(
          runtimeStateByConversation,
          sendingConversationId,
        );
        resetBackgroundRuntimeState(sendingConversationId, backgroundState);
      }
    }
  }

  async function handleUploadFiles(files: PendingUploadFile[]) {
    if (!files.length || isGenerating.value) {
      return;
    }

    mainView.value = "chat";
    pendingUploads.value = [...pendingUploads.value, ...files];
    emitToast({
      variant: "success",
      source: "upload",
      message: `已添加 ${files.length} 个附件到待发送列表。`,
    });
  }

  function handleRemovePendingUpload(index: number) {
    if (index < 0 || index >= pendingUploads.value.length) {
      return;
    }
    pendingUploads.value.splice(index, 1);
  }

  async function handleCancelGeneration() {
    if (!isGenerating.value) return;
    try {
      const hit = await cancelChatMessage(activeConversationId.value || null);
      if (!hit) {
        emitToast({
          variant: "warning",
          source: "cancel",
          message: "取消信号已发送，但未命中活动会话。",
        });
      }
    } catch (err) {
      console.error("Failed to cancel generation:", err);
    }
  }

  async function handleSendMessage(userText: string) {
    if (isGenerating.value) return;
    const text = userText.trim();
    const filesToSend = pendingUploads.value.slice();
    const textFiles = filesToSend.filter(isDocumentUploadFile);
    const imageFiles = filesToSend.filter(isImageUploadFile);
    if (!text && filesToSend.length === 0) return;

    mainView.value = "chat";
    resetPendingPromptState(activeRuntimeRefs);

    if (!activeConversationId.value) {
      const seedTitle = text || filesToSend[0]?.sourceName;
      const id = await createNewConversation(seedTitle);
      if (!id) return;
      activeConversationId.value = id;
      messages.value = [];
    }

    const sendingConversationId = activeConversationId.value;

    const uploadedAttachments: ChatAttachment[] = [
      ...toAttachmentMeta(textFiles, { includeDocumentContent: true }),
      ...toAttachmentMeta(imageFiles, { includeImageData: true }),
    ];
    const userMessage: ChatMessage = {
      role: "user",
      content: text,
      attachments: uploadedAttachments.length > 0 ? uploadedAttachments : undefined,
    };
    const nextMessages = [...messages.value, userMessage];
    const contextPlan = await prepareConversationContextForDispatch(
      sendingConversationId,
      nextMessages,
    );

    if (filesToSend.length > 0) {
      pendingUploads.value = [];
    }

    messages.value = contextPlan.messages;
    if (contextPlan.rewroteHistory) {
      await replaceConversationHistory(sendingConversationId, contextPlan.messages);
    } else {
      await persistMessage(userMessage, sendingConversationId);
    }
    await dispatchConversationMessages(
      sendingConversationId,
      contextPlan.messages,
      contextPlan.buildOptions,
    );
  }

  async function handleEditMessage(messageIndex: number, nextContent: string) {
    if (isGenerating.value) return;
    const conversationId = activeConversationId.value.trim();
    const trimmedContent = nextContent.trim();
    if (!conversationId || !trimmedContent) return;

    const originalMessage = messages.value[messageIndex];
    if (!originalMessage || originalMessage.role !== "user") {
      return;
    }

    mainView.value = "chat";
    resetPendingPromptState(activeRuntimeRefs);

    const nextMessages = [
      ...messages.value.slice(0, messageIndex),
      {
        ...originalMessage,
        content: trimmedContent,
      },
    ];

    try {
      const contextPlan = await prepareConversationContextForDispatch(conversationId, nextMessages);
      await replaceConversationHistory(conversationId, contextPlan.messages);
      messages.value = contextPlan.messages;
      toolExecutionLogs.value = [];
      pendingUploads.value = [];
      await dispatchConversationMessages(
        conversationId,
        contextPlan.messages,
        contextPlan.buildOptions,
      );
    } catch (err) {
      console.error("Failed to edit and resend message:", err);
      emitToast({
        variant: "error",
        source: "edit-message",
        message: "编辑消息失败：当前会话缺少可靠快照，请新开对话后继续。",
      });
    }
  }

  async function handlePendingQuestionSubmit(payload: AskUserAnswerSubmission) {
    if (pendingPermissionRequestId.value) {
      const action = extractPermissionActionFromAnswers(payload);
      if (!action) {
        emitToast({
          variant: "error",
          source: "permission",
          message: "未识别到权限操作，请重新选择允许/拒绝选项。",
        });
        return;
      }

      try {
        await submitPermissionDecision(
          activeConversationId.value || null,
          pendingPermissionRequestId.value,
          action,
        );
        resetPendingPromptState(activeRuntimeRefs);
      } catch (err) {
        console.error("Failed to submit permission decision:", err);
      }
      return;
    }

    await handleSendMessage(buildPendingQuestionReply(payload, "submit"));
  }

  async function handlePendingQuestionSkip() {
    if (pendingPermissionRequestId.value) {
      try {
        await submitPermissionDecision(
          activeConversationId.value || null,
          pendingPermissionRequestId.value,
          "deny_session",
        );
        resetPendingPromptState(activeRuntimeRefs);
      } catch (err) {
        console.error("Failed to submit permission denial:", err);
      }
      return;
    }

    await handleSendMessage(buildPendingQuestionReply(null, "skip"));
  }

  return {
    handleSendMessage,
    handleEditMessage,
    handleUploadFiles,
    handleRemovePendingUpload,
    handleCancelGeneration,
    handlePendingQuestionSubmit,
    handlePendingQuestionSkip,
  };
}
