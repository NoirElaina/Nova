import { defineStore } from "pinia";
import { computed, reactive } from "vue";
import type {
  AssistantTranscriptSegment,
  ChatMessage,
  ContextCompactSummary,
  ContextUsage,
  ConversationUsageSummary,
  NeedsUserInputPayload,
  ToolExecutionEntry,
  TurnCost,
} from "@/lib/chat-types";
import type { LiveTurnStage } from "@/features/chat/controllers/chat-controller-types";
import {
  getConversationUsage,
  loadConversationHistory,
  loadConversationToolLogs,
} from "@/features/chat/services/chat-api";
import { sanitizeConsecutiveAssistantMessages } from "@/features/chat/controllers/chat-message-helpers";
import {
  appendTranscriptReasoning,
  appendTranscriptText,
  appendTranscriptTool,
} from "@/features/chat/utils/assistant-transcript";
import { useConversationStore } from "./conversation";

export interface SessionRuntimeState {
  conversationId: string;
  messages: ChatMessage[];
  isGenerating: boolean;
  currentStage: LiveTurnStage;
  cognitiveState?: string;
  assistantResponse: string;
  assistantReasoning: string;
  assistantSegments: AssistantTranscriptSegment[];
  assistantTokenUsage?: number;
  assistantTurnCost?: TurnCost;
  toolExecutionLogs: ToolExecutionEntry[];
  pendingQuestion: NeedsUserInputPayload | null;
  pendingPermissionRequestId: string | null;
  currentTurnStartedAt: number | null;
  contextUsage?: ContextUsage;
  contextCompacts: ContextCompactSummary[];
  contextTokens: number;
  conversationUsage: ConversationUsageSummary | null;
  chatError: string | null;
  isCompacting: boolean;
}

export function createInitialSessionState(conversationId: string): SessionRuntimeState {
  return {
    conversationId,
    messages: [],
    isGenerating: false,
    currentStage: "processing",
    cognitiveState: "idle",
    assistantResponse: "",
    assistantReasoning: "",
    assistantSegments: [],
    assistantTokenUsage: undefined,
    assistantTurnCost: undefined,
    toolExecutionLogs: [],
    pendingQuestion: null,
    pendingPermissionRequestId: null,
    currentTurnStartedAt: null,
    contextUsage: undefined,
    contextCompacts: [],
    contextTokens: 0,
    conversationUsage: null,
    chatError: null,
    isCompacting: false,
  };
}

export const useAgentSessionStore = defineStore("agentSession", () => {
  const conversationStore = useConversationStore();
  const sessions = reactive<Record<string, SessionRuntimeState>>({});

  function getSession(convId?: string | null): SessionRuntimeState {
    const id = (convId ?? "").trim() || "__draft__";
    if (!sessions[id]) {
      sessions[id] = createInitialSessionState(id);
    }
    return sessions[id];
  }

  const activeSession = computed<SessionRuntimeState>(() => {
    return getSession(conversationStore.activeConversationId);
  });

  async function loadConversationMessages(conversationId: string) {
    if (!conversationId) return;
    const session = getSession(conversationId);
    try {
      const [saved, savedToolLogs] = await Promise.all([
        loadConversationHistory(conversationId),
        loadConversationToolLogs(conversationId).catch(() => []),
      ]);

      session.messages = sanitizeConsecutiveAssistantMessages(
        (saved || [])
          .filter(
            (msg) =>
              (msg.role === "user" || msg.role === "assistant") &&
              (!!msg.content || !!msg.reasoning || (msg.attachments?.length ?? 0) > 0 || (msg.cost?.transcriptSegments?.length ?? 0) > 0),
          )
          .map((msg, index) => ({
            id:
              msg.id != null && msg.id > 0
                ? String(msg.id)
                : `hist-${conversationId}-${index}`,
            role: msg.role as "user" | "assistant",
            content: msg.content,
            reasoning: msg.reasoning,
            attachments: msg.attachments,
            tokenUsage: msg.tokenUsage,
            cost: msg.cost,
            transcriptSegments: msg.cost?.transcriptSegments,
          })),
      );

      const logMap = new Map<string, ToolExecutionEntry>();
      for (const log of session.toolExecutionLogs) {
        logMap.set(log.id, log);
      }
      for (const log of (savedToolLogs || [])) {
        const existing = logMap.get(log.id);
        if (!existing || existing.status === "running" || log.status !== "running") {
          logMap.set(log.id, log);
        }
      }
      session.toolExecutionLogs = Array.from(logMap.values());

      try {
        session.conversationUsage = await getConversationUsage(conversationId);
      } catch {
        // non-blocking
      }
    } catch (err) {
      console.error(`Failed to load history for conversation ${conversationId}:`, err);
    }
  }

  function handleTurnStarted(_turnId: string, convId: string) {
    const session = getSession(convId);
    session.isGenerating = true;
    session.currentStage = "processing";
    session.cognitiveState = "assembling_context";
    session.assistantResponse = "";
    session.assistantReasoning = "";
    session.assistantSegments = [];
    session.currentTurnStartedAt = Date.now();
    session.chatError = null;
  }

  function handleStateChanged(state: string, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.cognitiveState = state;
    if (
      state === "assembling_context" ||
      state === "model_inference" ||
      state === "executing_tool" ||
      state === "verifying_workspace" ||
      state === "reflecting"
    ) {
      session.isGenerating = true;
    } else if (state === "turn_complete" || state === "failed" || state === "idle") {
      session.isGenerating = false;
    }
  }

  function handleThinkingDelta(delta: string, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.assistantReasoning += delta;
    appendTranscriptReasoning(session.assistantSegments, delta);
  }

  function handleTextDelta(delta: string, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.assistantResponse += delta;
    appendTranscriptText(session.assistantSegments, delta);
  }

  function handleToolRequested(callId: string, toolName: string, args: Record<string, unknown>, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.toolExecutionLogs.push({
      id: callId,
      toolName,
      status: "running",
      input: typeof args === "string" ? args : JSON.stringify(args, null, 2),
      result: "",
      startedAt: Date.now(),
    });
    appendTranscriptTool(session.assistantSegments, callId);
  }

  function handleToolCompleted(callId: string, _toolName: string, output: string, isError: boolean, _durationMs: number, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    const entry = session.toolExecutionLogs.find((e) => e.id === callId);
    if (entry) {
      entry.status = isError ? "error" : "completed";
      entry.result = output;
      entry.finishedAt = Date.now();
    }
  }

  function handleTokenUsage(usage: { input: number; output: number }, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.assistantTokenUsage = usage.input + usage.output;
  }

  async function handleTurnFinished(_turnId: string, _stopReason: string, convId?: string) {
    const targetId = convId || conversationStore.activeConversationId;
    const session = getSession(targetId);
    session.isGenerating = false;
    session.currentStage = "processing";
    session.cognitiveState = "idle";
    session.assistantResponse = "";
    session.assistantReasoning = "";
    session.assistantSegments = [];
    if (targetId && targetId !== "__draft__") {
      await loadConversationMessages(targetId);
    }
  }

  function handleTurnError(error: string, convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.isGenerating = false;
    session.chatError = error;
  }

  function dismissChatError(convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.chatError = null;
  }

  function handlePermissionRequested(
    requestId: string,
    _toolName: string,
    _payload: string,
    convId?: string,
  ) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.pendingPermissionRequestId = requestId;
    session.currentStage = "awaiting_approval";
  }

  function clearActiveTurnRuntime(convId?: string) {
    const session = convId ? getSession(convId) : activeSession.value;
    session.isGenerating = false;
    session.currentStage = "processing";
    session.assistantResponse = "";
    session.assistantReasoning = "";
    session.assistantSegments = [];
    session.assistantTokenUsage = undefined;
    session.assistantTurnCost = undefined;
    session.pendingQuestion = null;
    session.pendingPermissionRequestId = null;
    session.currentTurnStartedAt = null;
  }

  return {
    sessions,
    getSession,
    activeSession,
    loadConversationMessages,
    handleTurnStarted,
    handleStateChanged,
    handleThinkingDelta,
    handleTextDelta,
    handleToolRequested,
    handleToolCompleted,
    handleTokenUsage,
    handlePermissionRequested,
    handleTurnFinished,
    handleTurnError,
    dismissChatError,
    clearActiveTurnRuntime,
  };
});
