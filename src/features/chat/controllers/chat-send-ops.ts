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
  TurnCost,
} from "../../../lib/chat-types";
import {
  cancelChatMessage,
  sendChatMessage,
  submitPermissionDecision,
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
  isDocumentUploadFile,
  isImageUploadFile,
  toAttachmentMeta,
} from "./chat-message-helpers";

type SendOpsDeps = {
  activeConversationId: Ref<string>;
  isGenerating: Ref<boolean>;
  currentStage: Ref<LiveTurnStage>;
  messages: Ref<ChatMessage[]>;
  pendingUploads: Ref<PendingUploadFile[]>;
  pendingPermissionRequestId: Ref<string | null>;
  mainView: Ref<"chat" | "hooks" | "agent" | "schedule">;
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
    chatScreenRef,
    runtimeStateByConversation,
    activeRuntimeRefs,
    createNewConversation,
    persistMessage,
    refreshConversationFiles,
    resetBackgroundRuntimeState,
  } = deps;

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

    let uploadedAttachments: ChatAttachment[] = toAttachmentMeta(imageFiles);
    if (textFiles.length > 0) {
      try {
        const result = await upsertConversationRagDocuments(
          sendingConversationId,
          textFiles.map((file) => ({
            sourceName: file.sourceName,
            sourceType: "file",
            mimeType: file.mimeType,
            content: file.content,
          })),
        );

        if (result.added + result.updated <= 0 && imageFiles.length === 0) {
          emitToast({
            variant: "error",
            source: "upload",
            message: "文件上传失败，本轮未发送。",
          });
          return;
        }

        const rejectedNames = new Set(result.rejected.map((item) => item.sourceName));
        const acceptedTextFiles = textFiles.filter((file) => !rejectedNames.has(file.sourceName));
        uploadedAttachments = [
          ...toAttachmentMeta(acceptedTextFiles),
          ...toAttachmentMeta(imageFiles),
        ];
        await refreshConversationFiles(sendingConversationId);

        if (result.rejected.length > 0) {
          const detail = result.rejected
            .slice(0, 2)
            .map((item) => `${item.sourceName}(${item.reason})`)
            .join("；");
          emitToast({
            variant: "error",
            source: "upload",
            message: `部分文件上传失败：${detail}`,
          });
        }
      } catch (err) {
        emitToast({
          variant: "error",
          source: "upload",
          message: `文件上传失败，本轮未发送: ${String(err)}`,
        });
        return;
      }
    }

    if (filesToSend.length > 0) {
      pendingUploads.value = [];
    }

    if (activeConversationId.value !== sendingConversationId) {
      emitToast({
        variant: "info",
        source: "send",
        message: "会话已切换，本次发送已取消，请在当前会话重新发送。",
      });
      return;
    }

    const userMessage: ChatMessage = {
      role: "user",
      content: text,
      attachments: uploadedAttachments.length > 0 ? uploadedAttachments : undefined,
    };
    messages.value.push(userMessage);
    await persistMessage(userMessage, sendingConversationId);
    chatScreenRef.value?.scrollLastUserMessageToBottom();
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

    const rustMessages = messages.value.map((message) => buildModelMessage(message));

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
      const errorMessage: ChatMessage = { role: "assistant", content: `API Error: ${err}` };
      if (isActiveFailedConversation) {
        messages.value.push(errorMessage);
      }
      await persistMessage(errorMessage, sendingConversationId);

      if (isActiveFailedConversation) {
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
      emitToast({
        variant: "error",
        source: "cancel",
        message: `取消失败: ${String(err)}`,
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
        emitToast({
          variant: "error",
          source: "permission",
          message: `提交权限决策失败: ${String(err)}`,
        });
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
        emitToast({
          variant: "error",
          source: "permission",
          message: `提交权限拒绝失败: ${String(err)}`,
        });
      }
      return;
    }

    await handleSendMessage(buildPendingQuestionReply(null, "skip"));
  }

  return {
    handleSendMessage,
    handleUploadFiles,
    handleRemovePendingUpload,
    handleCancelGeneration,
    handlePendingQuestionSubmit,
    handlePendingQuestionSkip,
  };
}
