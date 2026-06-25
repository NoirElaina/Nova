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
  ContextCompactSummary,
  ContextUsage,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";
import { estimateTokens } from "../utils/session-memory";
import { buildToolTurnSummary } from "../utils/tool-activity-summary";
import {
  appendTranscriptReasoning,
  appendTranscriptText,
  appendTranscriptTool,
  buildAssistantTranscriptSegments,
} from "../utils/assistant-transcript";
import type {
  ConversationTurnRuntimeState,
  LiveTurnStage,
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
import { ackChatTurnStatus } from "../services/chat-api";

type PersistToolExecutionLog = (
  entry: ToolExecutionEntry,
  conversationId?: string,
) => void;

type TokenUsagePayload = {
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheCreationTokens?: number;
  totalInputTokens?: number;
  totalTokens?: number;
  cost?: Partial<TurnCost>;
  source?: string;
};

type ContextCompactPayload = {
  level?: string;
  reason?: string;
  beforeTokens?: number;
  afterTokens?: number;
  savedTokens?: number;
};

function parseTokenUsagePayload(raw?: string): TokenUsagePayload | null {
  if (!raw?.trim()) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as TokenUsagePayload;
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

function addDecimalStrings(left?: string, right?: string): string | undefined {
  if (!left && !right) return undefined;
  const a = parseDecimalString(left ?? "0");
  const b = parseDecimalString(right ?? "0");
  if (!a || !b) return right ?? left;
  const scale = Math.max(a.scale, b.scale);
  const total =
    a.units * 10n ** BigInt(scale - a.scale) +
    b.units * 10n ** BigInt(scale - b.scale);
  return formatDecimalUnits(total, scale);
}

function parseDecimalString(value: string): { units: bigint; scale: number } | null {
  const trimmed = value.trim();
  if (!/^\d+(?:\.\d+)?$/.test(trimmed)) return null;
  const [whole, fraction = ""] = trimmed.split(".");
  return {
    units: BigInt(`${whole}${fraction}`),
    scale: fraction.length,
  };
}

function formatDecimalUnits(units: bigint, scale: number): string {
  if (scale === 0) return units.toString();
  const divisor = 10n ** BigInt(scale);
  const whole = units / divisor;
  let fraction = (units % divisor).toString().padStart(scale, "0");
  fraction = fraction.replace(/0+$/, "");
  return fraction ? `${whole}.${fraction}` : whole.toString();
}

function mergeUsageCost(previous: TurnCost | undefined, incoming: TokenUsagePayload): Partial<TurnCost> {
  const cost = incoming.cost;
  return {
    cacheReadTokens:
      (previous?.cacheReadTokens ?? 0) +
      Math.max(0, incoming.cacheReadTokens ?? cost?.cacheReadTokens ?? 0),
    cacheCreationTokens:
      (previous?.cacheCreationTokens ?? 0) +
      Math.max(0, incoming.cacheCreationTokens ?? cost?.cacheCreationTokens ?? 0),
    billableInputTokens: cost?.billableInputTokens ?? previous?.billableInputTokens,
    inputCostUsd: addDecimalStrings(previous?.inputCostUsd, cost?.inputCostUsd),
    outputCostUsd: addDecimalStrings(previous?.outputCostUsd, cost?.outputCostUsd),
    cacheReadCostUsd: addDecimalStrings(previous?.cacheReadCostUsd, cost?.cacheReadCostUsd),
    cacheCreationCostUsd: addDecimalStrings(
      previous?.cacheCreationCostUsd,
      cost?.cacheCreationCostUsd,
    ),
    totalCostUsd: addDecimalStrings(previous?.totalCostUsd, cost?.totalCostUsd),
    pricingModel: cost?.pricingModel ?? previous?.pricingModel,
  };
}

function parseContextUsagePayload(raw?: string): ContextUsage | null {
  if (!raw?.trim()) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as ContextUsage;
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

function parseContextCompactPayload(raw?: string): ContextCompactPayload | null {
  if (!raw?.trim()) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as ContextCompactPayload;
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

function toContextCompactSummary(
  compact: ContextCompactPayload | null,
): ContextCompactSummary | null {
  const beforeTokens = compact?.beforeTokens ?? 0;
  const afterTokens = compact?.afterTokens ?? 0;
  const savedTokens =
    typeof compact?.savedTokens === "number"
      ? compact.savedTokens
      : Math.max(0, beforeTokens - afterTokens);

  if (savedTokens <= 0) {
    return null;
  }

  return {
    level: compact?.level,
    reason: compact?.reason,
    beforeTokens: typeof compact?.beforeTokens === "number" ? compact.beforeTokens : undefined,
    afterTokens: typeof compact?.afterTokens === "number" ? compact.afterTokens : undefined,
    savedTokens,
  };
}

function switchStage(state: ConversationTurnRuntimeState, nextStage: LiveTurnStage) {
  state.currentStage = nextStage;
}

type StreamOpsDeps = {
  activeRuntimeRefs: ActiveRuntimeRefs;
  activeRuntimeState: ConversationTurnRuntimeState;
  activeConversationId: Ref<string>;
  agentMode: Ref<AgentMode>;
  planMode: Ref<boolean>;
  messages: Ref<ChatMessage[]>;
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
    activeRuntimeRefs.assistantSegments.value = [];
    activeRuntimeRefs.assistantTokenUsage.value = undefined;
    activeRuntimeRefs.assistantTurnCost.value = undefined;
    activeRuntimeRefs.isGenerating.value = false;
    activeRuntimeRefs.currentStage.value = "processing";
    void ackChatTurnStatus(activeConversationId.value || null);
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

    const toolSummary = buildToolTurnSummary(
      state.toolExecutionLogs.filter((entry) => state.currentTurnToolIds.includes(entry.id)),
    );
    const transcriptSegments = buildAssistantTranscriptSegments(state.assistantSegments, {
      reasoning: finalReasoning,
      text: finalText,
    });

    if (finalText || finalReasoning) {
      const assistantMessage: ChatMessage = {
        role: "assistant",
        content: finalText || "（本轮没有返回可显示的文本内容）",
        reasoning: finalReasoning || undefined,
        transcriptSegments,
        tokenUsage: resolvedTokenUsage > 0 ? resolvedTokenUsage : undefined,
        cost: {
          ...buildAssistantCostForState(state, toolSummary),
          transcriptSegments,
        },
      };
      await persistMessage(assistantMessage, conversationId);
    }

    state.assistantResponse = "";
    state.assistantReasoning = "";
    state.assistantSegments = [];
    state.assistantTokenUsage = undefined;
    state.assistantTurnCost = undefined;
    state.isGenerating = false;
    state.currentStage = "processing";
    state.currentToolStartedAt = null;
    state.currentToolCalls = 0;
    state.currentToolDurationMs = 0;
    state.currentContextUsage = undefined;
    state.currentContextCompacts = [];
    state.currentContextTokens = 0;
    state.currentInputTokens = 0;
    state.currentOutputTokens = 0;
    if (!preservePendingPrompt) {
      state.pendingQuestion = null;
      state.pendingPermissionRequestId = null;
    }
    state.currentTurnToolIds = [];
    state.toolInputById.clear();
    state.toolNameById.clear();

    if (!preservePendingPrompt) {
      cleanupRuntimeStateIfIdle(runtimeStateByConversation, conversationId);
    }
    if (!preservePendingPrompt) {
      await ackChatTurnStatus(conversationId);
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

    const toolSummary = buildToolTurnSummary(
      activeRuntimeRefs.toolExecutionLogs.value.filter((entry) =>
        activeRuntimeRefs.currentTurnToolIds.value.includes(entry.id),
      ),
    );
    const transcriptSegments = buildAssistantTranscriptSegments(
      activeRuntimeRefs.assistantSegments.value,
      {
        reasoning: finalReasoning,
        text: finalText,
      },
    );

    const cost = buildAssistantCost(
      activeRuntimeRefs.currentInputTokens.value,
      activeRuntimeRefs.currentOutputTokens.value,
      activeRuntimeRefs.currentToolCalls.value,
      activeRuntimeRefs.currentToolDurationMs.value,
      activeRuntimeRefs.currentContextCompacts.value,
      toolSummary,
      activeRuntimeRefs.assistantTurnCost.value,
    );
    cost.transcriptSegments = transcriptSegments;
    activeRuntimeRefs.assistantTurnCost.value = cost;

    const assistantMessage: ChatMessage = {
      role: "assistant",
      content: finalText || "（本轮没有返回可显示的文本内容）",
      reasoning: finalReasoning || undefined,
      transcriptSegments,
      tokenUsage: resolvedTokenUsage > 0 ? resolvedTokenUsage : undefined,
      cost,
    };
    const conversationId = activeConversationId.value || null;
    messages.value.push(assistantMessage);
    void persistMessage(assistantMessage, conversationId ?? undefined).then(() =>
      ackChatTurnStatus(conversationId),
    );
    if (conversationId) {
      void persistConversationMemory(conversationId);
    }
    activeRuntimeRefs.assistantResponse.value = "";
    activeRuntimeRefs.assistantReasoning.value = "";
    activeRuntimeRefs.assistantSegments.value = [];
    activeRuntimeRefs.assistantTokenUsage.value = undefined;
    activeRuntimeRefs.isGenerating.value = false;
    activeRuntimeRefs.currentStage.value = "processing";
    if (activeConversationId.value) {
      runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
    }
  }

  function finalizeCancelledTurn(tokenUsage?: number) {
    const finalText = activeRuntimeRefs.assistantResponse.value.trim();
    const finalReasoning = activeRuntimeRefs.assistantReasoning.value.trim();
    const cancelledText = finalText ? `${finalText}\n\n（已取消当前轮）` : "已取消当前轮。";
    const fallbackTokenUsage = estimateTokens(cancelledText);
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

    const toolSummary = buildToolTurnSummary(
      activeRuntimeRefs.toolExecutionLogs.value.filter((entry) =>
        activeRuntimeRefs.currentTurnToolIds.value.includes(entry.id),
      ),
    );
    const transcriptSegments = buildAssistantTranscriptSegments(
      activeRuntimeRefs.assistantSegments.value,
      {
        reasoning: finalReasoning,
        text: finalText,
      },
    );
    const cost = buildAssistantCost(
      activeRuntimeRefs.currentInputTokens.value,
      activeRuntimeRefs.currentOutputTokens.value,
      activeRuntimeRefs.currentToolCalls.value,
      activeRuntimeRefs.currentToolDurationMs.value,
      activeRuntimeRefs.currentContextCompacts.value,
      toolSummary,
      activeRuntimeRefs.assistantTurnCost.value,
    );
    cost.transcriptSegments = transcriptSegments;

    const assistantMessage: ChatMessage = {
      role: "assistant",
      content: cancelledText,
      reasoning: finalReasoning || undefined,
      transcriptSegments,
      tokenUsage: resolvedTokenUsage > 0 ? resolvedTokenUsage : undefined,
      cost,
    };
    const conversationId = activeConversationId.value || null;
    messages.value.push(assistantMessage);
    void persistMessage(assistantMessage, conversationId ?? undefined).then(() =>
      ackChatTurnStatus(conversationId),
    );
    activeRuntimeRefs.assistantResponse.value = "";
    activeRuntimeRefs.assistantReasoning.value = "";
    activeRuntimeRefs.assistantSegments.value = [];
    activeRuntimeRefs.assistantTokenUsage.value = undefined;
    activeRuntimeRefs.assistantTurnCost.value = undefined;
    activeRuntimeRefs.isGenerating.value = false;
    activeRuntimeRefs.currentStage.value = "processing";
    if (activeConversationId.value) {
      runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
    }
  }

  function resetBackgroundRuntimeState(
    conversationId: string,
    state: ConversationTurnRuntimeState,
    preservePendingPrompt = false,
  ) {
    state.isGenerating = false;
    state.currentStage = "processing";
    state.assistantResponse = "";
    state.assistantReasoning = "";
    state.assistantSegments = [];
    state.assistantTokenUsage = undefined;
    state.assistantTurnCost = undefined;
    if (!preservePendingPrompt) {
      state.pendingPermissionRequestId = null;
      state.pendingQuestion = null;
    }
    state.currentToolStartedAt = null;
    state.currentToolCalls = 0;
    state.currentToolDurationMs = 0;
    state.currentContextUsage = undefined;
    state.currentContextCompacts = [];
    state.currentContextTokens = 0;
    state.currentInputTokens = 0;
    state.currentOutputTokens = 0;
    state.currentTurnToolIds = [];
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
      switchStage(state, "processing");
      state.assistantResponse += payload.text;
      state.assistantSegments = appendTranscriptText(state.assistantSegments, payload.text);
      return;
    }

    if (payload.type === "reasoning" && payload.text) {
      state.isGenerating = true;
      switchStage(state, "processing");
      state.assistantReasoning += payload.text;
      state.assistantSegments = appendTranscriptReasoning(state.assistantSegments, payload.text);
      return;
    }

    if (payload.type === "tool-use-start") {
      state.isGenerating = true;
      switchStage(state, "processing");
      state.currentToolCalls += 1;
      state.currentToolStartedAt = Date.now();

      const toolName = (payload.tool_use_name ?? "unknown").trim() || "unknown";
      const rawToolId = (payload.tool_use_id ?? "").trim();
      const toolId = rawToolId || `tool-${Date.now()}-${state.currentToolCalls}`;

      state.toolNameById.set(toolId, toolName);
      if (!state.toolInputById.has(toolId)) {
        state.toolInputById.set(toolId, "");
      }
      if (!state.currentTurnToolIds.includes(toolId)) {
        state.currentTurnToolIds = [...state.currentTurnToolIds, toolId];
      }
      state.assistantSegments = appendTranscriptTool(state.assistantSegments, toolId);

      startToolExecutionTraceInState(state, toolId, toolName);
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
            console.error("Failed to cancel malformed permission request:", err);
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
          console.error("Failed to auto-deny malformed permission request:", err);
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

      completeToolExecutionTraceInState(
        conversationId,
        state,
        toolId || null,
        toolName,
        result,
        payload.tool_is_error ? "error" : "completed",
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
          switchStage(state, "processing");
          const rendered = renderToolResult(result);
          const preview =
            rendered.length > 1200 ? `${rendered.slice(0, 1200)}\n...(truncated)` : rendered;
          state.assistantResponse += `\n${preview}\n`;
          state.assistantSegments = appendTranscriptText(
            state.assistantSegments,
            `\n${preview}\n`,
          );

          if (!isActive) {
            emitToast({
              variant: "info",
              source: "permission-request",
              message: `会话 ${conversationId} 需要继续输入，请切回该会话。`,
            });
          }
        }
      }
      return;
    }

    if (payload.type === "context-usage") {
      const usage = parseContextUsagePayload(payload.text);
      const usedTokens =
        typeof usage?.usedTokens === "number" && usage.usedTokens > 0
          ? usage.usedTokens
          : 0;
      state.currentContextUsage = usage
        ? {
            ...usage,
            usedTokens,
          }
        : undefined;
      state.currentContextTokens = usedTokens;
      return;
    }

    if (payload.type === "context-compact") {
      const compact = parseContextCompactPayload(payload.text);
      const summary = toContextCompactSummary(compact);
      if (!summary) {
        return;
      }
      state.isGenerating = true;
      switchStage(state, "compacting");
      state.currentContextCompacts = [...state.currentContextCompacts, summary];
      return;
    }

    if (payload.type === "token-usage") {
      const usage = parseTokenUsagePayload(payload.text);
      // 根据 Anthropic 文档：totalInput = inputTokens + cacheRead + cacheCreation
      const nextInputTokens =
        typeof usage?.totalInputTokens === "number" && usage.totalInputTokens > 0
          ? usage.totalInputTokens
          : typeof usage?.inputTokens === "number" && usage.inputTokens > 0
            ? usage.inputTokens
            : 0;
      const nextOutputTokens =
        typeof usage?.outputTokens === "number" && usage.outputTokens > 0
          ? usage.outputTokens
          : typeof payload.token_usage === "number" && payload.token_usage > 0
            ? payload.token_usage
            : 0;

      if (nextInputTokens > 0) {
        // inputTokens 是每轮请求的完整 prompt token 数（非增量），
        // 取最新一轮的值而非累加，避免多轮工具调用后 token 数成倍膨胀。
        state.currentInputTokens = nextInputTokens;
        state.currentContextTokens = nextInputTokens;
        state.currentContextUsage = {
          ...(state.currentContextUsage ?? { usedTokens: nextInputTokens }),
          usedTokens: nextInputTokens,
          source: "actual",
        };
      }
      if (nextOutputTokens > 0) {
        state.currentOutputTokens += nextOutputTokens;
        state.assistantTokenUsage = state.currentOutputTokens;
      }
      const usageCost = mergeUsageCost(state.assistantTurnCost, usage ?? {});
      state.assistantTurnCost = {
        ...usageCost,
        inputTokens: state.currentInputTokens,
        outputTokens: state.currentOutputTokens,
        toolCalls: state.currentToolCalls,
        toolDurationMs: state.currentToolDurationMs,
        contextCompacts: state.currentContextCompacts,
      };
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
        finalizeCancelledTurn(payload.token_usage);
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
        // 若流中断前已输出部分内容，提交为消息而非丢弃，与 cancel 行为保持一致。
        if (
          activeRuntimeRefs.assistantResponse.value.trim().length > 0 ||
          activeRuntimeRefs.assistantReasoning.value.trim().length > 0
        ) {
          finalizeOrStopTurn(undefined);
        } else {
          state.isGenerating = false;
          switchStage(state, "processing");
          state.assistantResponse = "";
          state.assistantReasoning = "";
          state.assistantSegments = [];
          state.assistantTokenUsage = undefined;
          state.assistantTurnCost = undefined;
          resetTurnRuntimeState(activeRuntimeRefs);
          if (activeConversationId.value) {
            runtimeStateByConversation.delete(normalizeConversationId(activeConversationId.value));
          }
          void ackChatTurnStatus(activeConversationId.value || null);
        }
      } else {
        resetBackgroundRuntimeState(conversationId, state);
        void ackChatTurnStatus(conversationId);
      }

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
