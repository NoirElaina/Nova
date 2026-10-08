import type { Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { emitToast, emitChatError } from "../../../lib/toast";
import { getRawErrorText } from "../../../lib/error-display";
import {
  buildPendingQuestionReply,
  extractPermissionActionFromAnswers,
} from "../../../lib/chat-payloads";
import type {
  AgentMode,
  AskUserAnswerSubmission,
  ChatAttachment,
  ChatMessage,
  PendingUploadFile,
} from "../../../lib/chat-types";
import {
  cancelChatMessage,
  replaceConversationHistory,
  sendModernAgentTurn,
  submitPermissionDecision,
  saveSessionFile,
} from "../services/chat-api";
import type { ChatScreenHandle } from "./chat-controller-types";
import {
  isDocumentUploadFile,
  isImageUploadFile,
  toAttachmentMeta,
} from "./chat-message-helpers";
import { useAgentSessionStore } from "@/stores/agentSession";
import { useComposerStore } from "@/stores/composer";

export type SendOpsDeps = {
  activeConversationId: Ref<string>;
  mainView: Ref<"chat" | "hooks" | "agent" | "schedule" | "settings">;
  agentMode: Ref<AgentMode>;
  pendingAgentBundleId: Ref<string | null>;
  chatScreenRef: Ref<ChatScreenHandle | null>;
  createNewConversation: (seedTitle?: string) => Promise<string | null>;
  refreshConversationFiles: (conversationId: string) => Promise<void>;
  finalizeActiveTurnOnError: () => Promise<void>;
};

export function createSendOperations(deps: SendOpsDeps) {
  const {
    activeConversationId,
    mainView,
    pendingAgentBundleId,
    chatScreenRef,
    createNewConversation,
    refreshConversationFiles,
    finalizeActiveTurnOnError,
  } = deps;

  const sessionStore = useAgentSessionStore();
  const composerStore = useComposerStore();

  async function dispatchConversationMessages(
    sendingConversationId: string,
    nextMessages: ChatMessage[],
  ) {
    if (activeConversationId.value !== sendingConversationId) {
      emitToast({
        variant: "info",
        source: "send",
        message: "会话已切换，本次发送已取消，请在当前会话重新发送。",
      });
      return;
    }

    const turnId = `turn-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    sessionStore.handleTurnStarted(turnId, sendingConversationId);
    void chatScreenRef.value?.scrollLiveAssistantIntoView();

    const lastMessage = nextMessages[nextMessages.length - 1];
    const userPrompt = lastMessage?.role === "user" ? lastMessage.content : "";

    try {
      await sendModernAgentTurn(sendingConversationId, userPrompt);
    } catch (err: unknown) {
      console.error("Chat error:", err);
      const raw = getRawErrorText(err);
      emitChatError({
        source: "send",
        message: raw || "消息发送失败，请检查后端日志后重试。",
      });

      const session = sessionStore.getSession(sendingConversationId);
      if (
        session.assistantResponse.trim().length > 0 ||
        session.assistantReasoning.trim().length > 0
      ) {
        await finalizeActiveTurnOnError();
      }
      sessionStore.handleTurnError(raw || "Turn execution failed", sendingConversationId);
    }
  }

  async function handleUploadFiles(files: PendingUploadFile[]) {
    if (!files.length || sessionStore.activeSession.isGenerating) {
      return;
    }

    mainView.value = "chat";
    composerStore.addUploads(files);
    emitToast({
      variant: "success",
      source: "upload",
      message: `已添加 ${files.length} 个附件到待发送列表。`,
    });
  }

  function handleRemovePendingUpload(index: number) {
    composerStore.removeUpload(index);
  }

  async function handleCancelGeneration() {
    if (!sessionStore.activeSession.isGenerating) return;
    try {
      // 1. 立即前端响应：设置生成状态为 false，将正在运行的工具直接标记为已取消，给用户立竿见影的反馈
      sessionStore.activeSession.isGenerating = false;
      sessionStore.activeSession.currentStage = "processing";
      sessionStore.activeSession.cognitiveState = "idle";
      for (const tool of sessionStore.activeSession.toolExecutionLogs) {
        if (tool.status === "running") {
          tool.status = "cancelled";
        }
      }

      // 2. 向后端发送强制取消指令
      const hit = await cancelChatMessage(activeConversationId.value || null);
      if (!hit) {
        // 兜底：若带 ID 未命中，尝试全局取消
        await cancelChatMessage(null);
      }
    } catch (err) {
      console.error("Failed to cancel generation:", err);
    }
  }

  async function handleSendMessage(userText: string) {
    if (sessionStore.activeSession.isGenerating) return;
    const text = userText.trim();
    const filesToSend = composerStore.pendingUploads.slice();
    const textFiles = filesToSend.filter(isDocumentUploadFile);
    const imageFiles = filesToSend.filter(isImageUploadFile);
    if (!text && filesToSend.length === 0) return;

    mainView.value = "chat";
    sessionStore.clearActiveTurnRuntime();

    if (!activeConversationId.value) {
      const seedTitle = text || filesToSend[0]?.sourceName;
      const id = await createNewConversation(seedTitle);
      if (!id) return;
      activeConversationId.value = id;
      sessionStore.activeSession.messages = [];
      // 智能体页「启用」暂存的智能体：对话真正创建时才挂载（延迟创建语义），挂载后清空暂存。
      const pendingAgentId = pendingAgentBundleId.value;
      if (pendingAgentId) {
        try {
          await invoke("set_conversation_agent", {
            conversationId: id,
            bundleId: pendingAgentId,
          });
          window.dispatchEvent(new CustomEvent("agent-bundle-changed"));
        } catch (err) {
          console.error("Failed to attach pending agent to new conversation:", err);
        } finally {
          pendingAgentBundleId.value = null;
        }
      }
    }

    const sendingConversationId = activeConversationId.value;

    // 所有文档文件都存为会话文件；纯文本文件内容存入 attachment.content，发送时注入 prompt
    const documentAttachments: ChatAttachment[] = [];
    for (const file of textFiles) {
      try {
        const meta = await saveSessionFile(
          sendingConversationId,
          file.sourceName,
          file.content,
          file.rawBytes,
        );
        documentAttachments.push({
          sourceName: file.sourceName,
          mimeType: file.mimeType,
          size: file.size,
          kind: "document",
          content: file.content ?? undefined,
          sessionFilePath: meta.filename || undefined,
        });
      } catch (err) {
        console.error("Failed to save session file:", err);
        emitToast({
          variant: "error",
          source: "upload",
          message: `保存会话文件失败：${file.sourceName}`,
        });
      }
    }

    if (textFiles.length > 0) {
      await refreshConversationFiles(sendingConversationId);
    }

    const imageAttachments = toAttachmentMeta(imageFiles, { includeImageData: true });
    const uploadedAttachments = [...documentAttachments, ...imageAttachments];
    const userMessage: ChatMessage = {
      id: `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      role: "user",
      content: text,
      attachments: uploadedAttachments.length > 0 ? uploadedAttachments : undefined,
      createdAt: Date.now(),
    };

    const nextMessages = [...sessionStore.activeSession.messages, userMessage];

    if (filesToSend.length > 0) {
      composerStore.clearUploads();
    }

    sessionStore.activeSession.messages = nextMessages;
    await dispatchConversationMessages(sendingConversationId, nextMessages);

  }

  async function handleEditMessage(
    payload: { index: number; content: string; id?: string },
  ) {
    if (sessionStore.activeSession.isGenerating) return;
    const conversationId = activeConversationId.value.trim();
    const trimmedContent = payload.content.trim();
    if (!conversationId || !trimmedContent) return;

    const currentMessages = sessionStore.activeSession.messages;
    let messageIndex = -1;
    if (payload.id) {
      messageIndex = currentMessages.findIndex((item) => item.id === payload.id);
    }
    if (messageIndex < 0) {
      messageIndex = payload.index;
    }

    const originalMessage = currentMessages[messageIndex];
    if (!originalMessage || originalMessage.role !== "user") {
      return;
    }

    if (
      payload.id &&
      originalMessage.id &&
      originalMessage.id !== payload.id
    ) {
      emitToast({
        variant: "error",
        source: "edit-message",
        message: "无法定位要编辑的消息，请刷新会话后重试。",
      });
      return;
    }

    mainView.value = "chat";
    sessionStore.clearActiveTurnRuntime();

    const nextMessages = [
      ...currentMessages.slice(0, messageIndex),
      {
        ...originalMessage,
        content: trimmedContent,
      },
    ];

    try {
      await replaceConversationHistory(conversationId, currentMessages.slice(0, messageIndex));
      sessionStore.activeSession.messages = nextMessages;

      sessionStore.activeSession.toolExecutionLogs = [];
      composerStore.clearUploads();
      await dispatchConversationMessages(conversationId, nextMessages);
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
    const pendingReqId = sessionStore.activeSession.pendingPermissionRequestId;
    if (pendingReqId) {
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
          pendingReqId,
          action,
        );
        sessionStore.activeSession.pendingPermissionRequestId = null;
      } catch (err) {
        console.error("Failed to submit permission decision:", err);
      }
      return;
    }

    await handleSendMessage(buildPendingQuestionReply(payload, "submit"));
  }

  async function handlePendingQuestionSkip() {
    const pendingReqId = sessionStore.activeSession.pendingPermissionRequestId;
    if (pendingReqId) {
      try {
        await submitPermissionDecision(
          activeConversationId.value || null,
          pendingReqId,
          "deny_once",
        );
        sessionStore.activeSession.pendingPermissionRequestId = null;
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
