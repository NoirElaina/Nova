import type { Ref } from "vue";
import { emitToast } from "../../../lib/toast";
import {
  parseNeedsUserInput,
  parsePlanModeChange,
  renderToolResult,
} from "../../../lib/chat-payloads";
import type {
  AgentMode,
  ChatMessage,
  ChatMessageEvent,
  ToolExecutionEntry,
} from "../../../lib/chat-types";
import { summarizeToolInfo } from "../utils/tool-info";
import { estimateTokens } from "../utils/session-memory";
import type {
  ChatScreenHandle,
  ConversationTurnRuntimeState,
} from "./chat-controller-types";
import type { ActiveRuntimeRefs } from "./chat-runtime-state";
import {
  cleanupRuntimeStateIfIdle,
  ensureRuntimeState,
  normalizeConversationId,
  resetPendingPromptState,
  resetToolTrackingState,
  resetTurnRuntimeState,
} from "./chat-runtime-state";
import {
  appendToolExecutionInputInState,
  completeToolExecutionTraceInState,
  latestRunningToolExecutionIdByName,
  markRunningToolExecutionsInState,
  startToolExecutionTraceInState,
} from "./chat-tool-execution";
import {
  buildAssistantCost,
  buildAssistantCostForState,
  shouldPreservePendingPromptOnStop,
} from "./chat-message-helpers";

type PersistToolExecutionLog = (
  entry: ToolExecutionEntry,
  conversationId?: string,
) => void;

type StreamOpsDeps = {
  activeRuntimeRefs: ActiveRuntimeRefs;
  activeRuntimeState: ConversationTurnRuntimeState;
  activeConversationId: Ref<string>;
  agentMode: Ref<AgentMode>;
  planMode: Ref<boolean>;
  messages: Ref<ChatMessage[]>;
  chatScreenRef: Ref<ChatScreenHandle | null>;
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>;
  persistMessage: (message: ChatMessage, conversationId?: string) => Promise<void>;
  persistConversationMemory: (conversationId: string) => Promise<void>;
  persistToolExecutionLog: PersistToolExecutionLog;
  cancelActiveConversation: () => Promise<unknown>;
  submitPermissionDecision: (
    conversationId: string | null,
    requestId: string,
    action: "deny_session",
  ) => Promise<boolean>;
};

export function createChatStreamOperations(deps: StreamOpsDeps) {
  const {
    activeRuntimeRefs,
    activeRuntimeState,
    activeConversationId,
    agentMode,
    planMode,
    messages,
    chatScreenRef,
    runtimeStateByConversation,
    persistMessage,
    persistConversationMemory,
    persistToolExecutionLog,
    cancelActiveConversation,
    submitPermissionDecision,
  } = deps;

  function finalizeOrStopTurn(tokenUsage?: number) {
    if (
      activeRuntimeRefs.assistantResponse.value.trim().length > 0 ||
      activeRuntimeRefs.assistantReasoning.value.trim().length > 0
    ) {
      finalizeAssistantTurn(tokenUsage);
      return;
    }
    activeRuntimeRefs.assistantResponse.value = "";
    activeRuntimeRefs.assistantReasoning.value = "";
    activeRuntimeRefs.assistantTokenUsage.value = undefined;
    activeRuntimeRefs.assistantTurnCost.value = undefined;
    activeRuntimeRefs.isGenerating.value = false;
  }

  async function finalizeBackgroundTurn(
    conversationId: string,
    state: ConversationTurnRuntimeState,
    tokenUsage?: number,
    preservePendingPrompt = false,
  ) {
    const finalText = state.assistantResponse.trim();
    const finalReasoning = state.assistantReasoning.trim();
    const fallbackTokenUsage = finalText ? estimateTokens(finalText) : 0;
    const resolvedTokenUsage =
      typeof tokenUsage === "number" && tokenUsage > 0
        ? tokenUsage
        : typeof state.assistantTokenUsage === "number" && state.assistantTokenUsage > 0
          ? state.assistantTokenUsage
          : fallbackTokenUsage;

    if (state.currentOutputTokens <= 0 && resolvedTokenUsage > 0) {
      state.currentOutputTokens = resolvedTokenUsage;
    }

    if (finalText || finalReasoning) {
      const assistantMessage: ChatMessage = {
        role: "assistant",
        content: finalText || "（本轮没有返回可显示的文本内容）",
        reasoning: finalReasoning || undefined,
        tokenUsage: resolvedTokenUsage > 0 ? resolvedTokenUsage : undefined,
        cost: buildAssistantCostForState(state),
      };
      await persistMessage(assistantMessage, conversationId);
    }

    state.assistantResponse = "";
    state.assistantReasoning = "";
    state.assistantTokenUsage = undefined;
    state.assistantTurnCost = undefined;
    state.isGenerating = false;
    state.currentToolStartedAt = null;
    state.currentToolCalls = 0;
    state.currentToolDurationMs = 0;
    state.currentInputTokens = 0;
    state.currentOutputTokens = 0;
    if (!preservePendingPrompt) {
      state.pendingQuestion = null;
      state.pendingPermissionRequestId = null;
    }
    state.toolInputById.clear();
    state.toolNameById.clear();

    if (!preservePendingPrompt) {
      cleanupRuntimeStateIfIdle(runtimeStateByConversation, conversationId);
    }
  }

  function finalizeAssistantTurn(tokenUsage?: number) {
    const finalText = activeRuntimeRefs.assistantResponse.value.trim();
    const finalReasoning = activeRuntimeRefs.assistantReasoning.value.trim();
    const fallbackTokenUsage = finalText ? estimateTokens(finalText) : 0;
    const resolvedTokenUsage =
      typeof tokenUsage === "number" && tokenUsage > 0
        ? tokenUsage
        : typeof activeRuntimeRefs.assistantTokenUsage.value === "number" &&
            activeRuntimeRefs.assistantTokenUsage.value > 0
          ? activeRuntimeRefs.assistantTokenUsage.value
          : fallbackTokenUsage;

    if (activeRuntimeRefs.currentOutputTokens.value <= 0 && resolvedTokenUsage > 0) {
      activeRuntimeRefs.currentOutputTokens.value = resolvedTokenUsage;
    }

    const cost = buildAssistantCost(
      activeRuntimeRefs.currentInputTokens.value,
      activeRuntimeRefs.currentOutputTokens.value,
      activeRuntimeRefs.currentToolCalls.value,
      activeRuntimeRefs.currentToolDurationMs.value,
    );
    activeRuntimeRefs.assistantTurnCost.value = cost;

    const assistantMessage: ChatMessage = {
      role: "assistant",
      content: finalText || "（本轮没有返回可显示的文本内容）",
      reasoning: finalReasoning || undefined,
      tokenUsage: resolvedTokenUsage > 0 ? resolvedTokenUsage : undefined,
      cost,
    };
    messages.value.push(assistantMessage);
    void persistMessage(assistantMessage);
    void persistConversationMemory(activeConversationId.value);
    activeRuntimeRefs.assistantResponse.value = "";
    activeRuntimeRefs.assistantReasoning.value = "";
    activeRuntimeRefs.assistantTokenUsage.value = undefined;
    activeRuntimeRefs.isGenerating.value = false;
    if (activeConversationId.value) {
      runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
    }
    chatScreenRef.value?.scrollToBottom();
  }

  function resetBackgroundRuntimeState(
    conversationId: string,
    state: ConversationTurnRuntimeState,
    preservePendingPrompt = false,
  ) {
    state.isGenerating = false;
    state.assistantResponse = "";
    state.assistantReasoning = "";
    state.assistantTokenUsage = undefined;
    state.assistantTurnCost = undefined;
    if (!preservePendingPrompt) {
      state.pendingPermissionRequestId = null;
      state.pendingQuestion = null;
    }
    state.currentToolStartedAt = null;
    state.currentToolCalls = 0;
    state.currentToolDurationMs = 0;
    state.currentInputTokens = 0;
    state.currentOutputTokens = 0;
    state.toolInputById.clear();
    state.toolNameById.clear();

    if (!preservePendingPrompt) {
      cleanupRuntimeStateIfIdle(runtimeStateByConversation, conversationId);
    }
  }

  async function handleChatStreamEvent(
    conversationId: string,
    payload: ChatMessageEvent,
    mode: "active" | "background",
  ) {
    const isActive = mode === "active";
    const state = isActive
      ? activeRuntimeState
      : ensureRuntimeState(runtimeStateByConversation, conversationId);

    if (payload.type === "text" && payload.text) {
      state.isGenerating = true;
      state.assistantResponse += payload.text;
      if (isActive) {
        chatScreenRef.value?.scrollToBottom();
      }
      return;
    }

    if (payload.type === "reasoning" && payload.text) {
      state.isGenerating = true;
      state.assistantReasoning += payload.text;
      if (isActive) {
        chatScreenRef.value?.scrollToBottom();
      }
      return;
    }

    if (payload.type === "tool-use-start") {
      state.isGenerating = true;
      state.currentToolCalls += 1;
      state.currentToolStartedAt = Date.now();

      const toolName = (payload.tool_use_name ?? "unknown").trim() || "unknown";
      const rawToolId = (payload.tool_use_id ?? "").trim();
      const toolId = rawToolId || `tool-${Date.now()}-${state.currentToolCalls}`;

      state.toolNameById.set(toolId, toolName);
      if (!state.toolInputById.has(toolId)) {
        state.toolInputById.set(toolId, "");
      }

      startToolExecutionTraceInState(state, toolId, toolName);

      if (isActive) {
        state.assistantResponse += `\n> Using tool: ${toolName}...\n`;
        chatScreenRef.value?.scrollToBottom();
      }
      return;
    }

    if (payload.type === "tool-json-delta") {
      const toolId = (payload.tool_use_id ?? "").trim();
      if (toolId && payload.tool_use_input) {
        const prev = state.toolInputById.get(toolId) ?? "";
        state.toolInputById.set(toolId, prev + payload.tool_use_input);
        appendToolExecutionInputInState(state, toolId, payload.tool_use_input);
      }
      return;
    }

    if (payload.type === "permission-request") {
      const requestId = (payload.tool_use_id ?? "").trim();
      const promptPayload = (payload.text ?? "").trim();
      const parsed = parseNeedsUserInput(promptPayload);

      if (!requestId) {
        emitToast({
          variant: "error",
          source: "permission-request",
          message: isActive
            ? "收到权限请求但缺少 request_id，已尝试取消当前回合。"
            : `会话 ${conversationId} 收到异常权限请求，无法继续处理。`,
        });

        if (isActive) {
          void cancelActiveConversation().catch((err) => {
            emitToast({
              variant: "error",
              source: "permission-request",
              message: `取消异常权限请求失败: ${String(err)}`,
            });
          });
          resetPendingPromptState(activeRuntimeRefs);
        }
        return;
      }

      if (!parsed) {
        emitToast({
          variant: "error",
          source: "permission-request",
          message: isActive
            ? "收到权限请求但参数无效，已自动拒绝该请求。"
            : `会话 ${conversationId} 收到异常权限请求，已自动拒绝。`,
        });
        void submitPermissionDecision(
          isActive ? activeConversationId.value || null : conversationId,
          requestId,
          "deny_session",
        ).catch((err) => {
          emitToast({
            variant: "error",
            source: "permission-request",
            message: isActive
              ? `自动拒绝权限请求失败: ${String(err)}`
              : `会话 ${conversationId} 自动拒绝权限请求失败: ${String(err)}`,
          });
        });
        if (isActive) {
          resetPendingPromptState(activeRuntimeRefs);
        }
        return;
      }

      state.pendingPermissionRequestId = requestId;
      state.pendingQuestion = parsed;
      if (!isActive) {
        state.isGenerating = false;
        emitToast({
          variant: "info",
          source: "permission-request",
          message: `会话 ${conversationId} 需要权限确认，请切回该会话处理。`,
        });
      } else {
        chatScreenRef.value?.scrollToBottom();
      }
      return;
    }

    if (payload.type === "tool-result") {
      if (state.currentToolStartedAt) {
        state.currentToolDurationMs += Math.max(0, Date.now() - state.currentToolStartedAt);
        state.currentToolStartedAt = null;
      }

      const rawToolId = (payload.tool_use_id ?? "").trim();
      const fallbackToolName = (payload.tool_use_name ?? "").trim();
      const toolName =
        fallbackToolName ||
        (rawToolId ? state.toolNameById.get(rawToolId) : undefined) ||
        "unknown";
      const toolId =
        rawToolId ||
        latestRunningToolExecutionIdByName(state.toolExecutionLogs, toolName) ||
        "";
      const streamedInput = toolId ? state.toolInputById.get(toolId) ?? "" : "";
      const fallbackInput = (payload.tool_use_input ?? "").trim();
      const rawInput = streamedInput.trim().length > 0 ? streamedInput : fallbackInput;
      const result = (payload.tool_result ?? "").trim();

      if (isActive) {
        const info = summarizeToolInfo(toolName, rawInput);
        if (info) {
          state.assistantResponse += `\n> Tool info: ${info}\n`;
        }
        state.assistantResponse += `\n> Tool done: ${toolName}\n`;
      }

      completeToolExecutionTraceInState(
        conversationId,
        state,
        toolId || null,
        toolName,
        result,
        "completed",
        persistToolExecutionLog,
        rawInput,
      );

      if (toolId) {
        state.toolInputById.delete(toolId);
        state.toolNameById.delete(toolId);
      }

      if (result) {
        if (isActive) {
          const planModeChange = parsePlanModeChange(result);
          if (planModeChange) {
            const nextIsPlanMode = planModeChange.mode === "plan";
            planMode.value = nextIsPlanMode;
            agentMode.value = nextIsPlanMode ? "plan" : "agent";
          }
        }

        const needsUserInput = parseNeedsUserInput(result);
        if (needsUserInput) {
          state.pendingPermissionRequestId = null;
          state.pendingQuestion = needsUserInput;
          state.isGenerating = false;
          const rendered = renderToolResult(result);
          const preview =
            rendered.length > 1200 ? `${rendered.slice(0, 1200)}\n...(truncated)` : rendered;
          state.assistantResponse += `\n${preview}\n`;

          if (!isActive) {
            emitToast({
              variant: "info",
              source: "permission-request",
              message: `会话 ${conversationId} 需要继续输入，请切回该会话。`,
            });
          }
        }
      }

      if (isActive) {
        chatScreenRef.value?.scrollToBottom();
      }
      return;
    }

    if (payload.type === "token-usage") {
      state.assistantTokenUsage = payload.token_usage;
      state.currentOutputTokens = payload.token_usage ?? state.currentOutputTokens;
      return;
    }

    if (payload.type !== "stop") {
      return;
    }

    const stopReason = payload.stop_reason ?? "";
    const turnState = payload.turn_state ?? "";

    if (turnState === "cancelled" || stopReason === "cancelled") {
      markRunningToolExecutionsInState(
        conversationId,
        state,
        "cancelled",
        persistToolExecutionLog,
      );

      if (isActive) {
        finalizeOrStopTurn(payload.token_usage);
        resetTurnRuntimeState(activeRuntimeRefs);
        if (activeConversationId.value) {
          runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
        }
      } else {
        resetBackgroundRuntimeState(conversationId, state);
      }
      return;
    }

    if (turnState === "error") {
      markRunningToolExecutionsInState(
        conversationId,
        state,
        "error",
        persistToolExecutionLog,
      );

      if (isActive) {
        state.isGenerating = false;
        state.assistantResponse = "";
        state.assistantReasoning = "";
        state.assistantTokenUsage = undefined;
        state.assistantTurnCost = undefined;
        resetTurnRuntimeState(activeRuntimeRefs);
        if (activeConversationId.value) {
          runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
        }
      } else {
        resetBackgroundRuntimeState(conversationId, state);
      }

      const detail = (payload.text ?? "").trim();
      emitToast({
        variant: "error",
        source: "chat-stream",
        message:
          detail ||
          (isActive
            ? `Provider error: ${stopReason || "unknown"}`
            : `会话 ${conversationId} 回复失败: ${stopReason || "unknown"}`),
      });
      return;
    }

    const shouldFinalize =
      turnState === "completed" ||
      turnState === "awaiting_user_input" ||
      turnState === "needs_user_input" ||
      turnState === "stop_hook_prevented" ||
      stopReason === "stop_hook_prevented" ||
      stopReason === "needs_user_input";

    if (!shouldFinalize) {
      return;
    }

    const preservePendingPrompt =
      shouldPreservePendingPromptOnStop(turnState, stopReason) ||
      !!state.pendingPermissionRequestId ||
      !!state.pendingQuestion;
    markRunningToolExecutionsInState(
      conversationId,
      state,
      "completed",
      persistToolExecutionLog,
    );

    if (isActive) {
      finalizeOrStopTurn(payload.token_usage);

      if (!preservePendingPrompt) {
        resetTurnRuntimeState(activeRuntimeRefs);
      } else {
        resetToolTrackingState(activeRuntimeRefs);
      }

      if (activeConversationId.value && !preservePendingPrompt) {
        runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
      }
      return;
    }

    await finalizeBackgroundTurn(
      conversationId,
      state,
      payload.token_usage,
      preservePendingPrompt,
    );
  }

  return {
    finalizeOrStopTurn,
    resetBackgroundRuntimeState,
    handleChatStreamEvent,
  };
}
