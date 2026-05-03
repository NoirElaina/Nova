import type { Ref } from "vue";
import type {
  NeedsUserInputPayload,
  ContextUsage,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";
import type { ConversationTurnRuntimeState } from "./chat-controller-types";

function cloneContextUsage(usage: ContextUsage | undefined): ContextUsage | undefined {
  return usage ? { ...usage } : undefined;
}

export type ActiveRuntimeRefs = {
  isGenerating: Ref<boolean>;
  assistantResponse: Ref<string>;
  assistantReasoning: Ref<string>;
  assistantTokenUsage: Ref<number | undefined>;
  assistantTurnCost: Ref<TurnCost | undefined>;
  pendingQuestion: Ref<NeedsUserInputPayload | null>;
  pendingPermissionRequestId: Ref<string | null>;
  currentToolStartedAt: Ref<number | null>;
  currentToolCalls: Ref<number>;
  currentToolDurationMs: Ref<number>;
  currentContextUsage: Ref<ContextUsage | undefined>;
  currentContextTokens: Ref<number>;
  currentInputTokens: Ref<number>;
  currentOutputTokens: Ref<number>;
  toolExecutionLogs: Ref<ToolExecutionEntry[]>;
  currentTurnToolIds: Ref<string[]>;
  toolInputById: Map<string, string>;
  toolNameById: Map<string, string>;
};

export function bindActiveRuntimeState(active: ActiveRuntimeRefs): ConversationTurnRuntimeState {
  return {
    get isGenerating() {
      return active.isGenerating.value;
    },
    set isGenerating(value: boolean) {
      active.isGenerating.value = value;
    },
    get assistantResponse() {
      return active.assistantResponse.value;
    },
    set assistantResponse(value: string) {
      active.assistantResponse.value = value;
    },
    get assistantReasoning() {
      return active.assistantReasoning.value;
    },
    set assistantReasoning(value: string) {
      active.assistantReasoning.value = value;
    },
    get assistantTokenUsage() {
      return active.assistantTokenUsage.value;
    },
    set assistantTokenUsage(value: number | undefined) {
      active.assistantTokenUsage.value = value;
    },
    get assistantTurnCost() {
      return active.assistantTurnCost.value;
    },
    set assistantTurnCost(value: TurnCost | undefined) {
      active.assistantTurnCost.value = value;
    },
    get pendingQuestion() {
      return active.pendingQuestion.value;
    },
    set pendingQuestion(value: NeedsUserInputPayload | null) {
      active.pendingQuestion.value = value;
    },
    get pendingPermissionRequestId() {
      return active.pendingPermissionRequestId.value;
    },
    set pendingPermissionRequestId(value: string | null) {
      active.pendingPermissionRequestId.value = value;
    },
    get currentToolStartedAt() {
      return active.currentToolStartedAt.value;
    },
    set currentToolStartedAt(value: number | null) {
      active.currentToolStartedAt.value = value;
    },
    get currentToolCalls() {
      return active.currentToolCalls.value;
    },
    set currentToolCalls(value: number) {
      active.currentToolCalls.value = value;
    },
    get currentToolDurationMs() {
      return active.currentToolDurationMs.value;
    },
    set currentToolDurationMs(value: number) {
      active.currentToolDurationMs.value = value;
    },
    get currentContextUsage() {
      return active.currentContextUsage.value;
    },
    set currentContextUsage(value: ContextUsage | undefined) {
      active.currentContextUsage.value = value;
    },
    get currentContextTokens() {
      return active.currentContextTokens.value;
    },
    set currentContextTokens(value: number) {
      active.currentContextTokens.value = value;
    },
    get currentInputTokens() {
      return active.currentInputTokens.value;
    },
    set currentInputTokens(value: number) {
      active.currentInputTokens.value = value;
    },
    get currentOutputTokens() {
      return active.currentOutputTokens.value;
    },
    set currentOutputTokens(value: number) {
      active.currentOutputTokens.value = value;
    },
    get toolExecutionLogs() {
      return active.toolExecutionLogs.value;
    },
    set toolExecutionLogs(value: ToolExecutionEntry[]) {
      active.toolExecutionLogs.value = value;
    },
    get currentTurnToolIds() {
      return active.currentTurnToolIds.value;
    },
    set currentTurnToolIds(value: string[]) {
      active.currentTurnToolIds.value = [...value];
    },
    get toolInputById() {
      return active.toolInputById;
    },
    set toolInputById(value: Map<string, string>) {
      active.toolInputById.clear();
      for (const [id, input] of value.entries()) {
        active.toolInputById.set(id, input);
      }
    },
    get toolNameById() {
      return active.toolNameById;
    },
    set toolNameById(value: Map<string, string>) {
      active.toolNameById.clear();
      for (const [id, name] of value.entries()) {
        active.toolNameById.set(id, name);
      }
    },
  };
}

export function createEmptyRuntimeState(): ConversationTurnRuntimeState {
  return {
    isGenerating: false,
    assistantResponse: "",
    assistantReasoning: "",
    assistantTokenUsage: undefined,
    assistantTurnCost: undefined,
    pendingQuestion: null,
    pendingPermissionRequestId: null,
    currentToolStartedAt: null,
    currentToolCalls: 0,
    currentToolDurationMs: 0,
    currentContextUsage: undefined,
    currentContextTokens: 0,
    currentInputTokens: 0,
    currentOutputTokens: 0,
    toolExecutionLogs: [],
    currentTurnToolIds: [],
    toolInputById: new Map<string, string>(),
    toolNameById: new Map<string, string>(),
  };
}

export function normalizeConversationId(conversationId?: string | null): string {
  const normalized = (conversationId ?? "").trim();
  return normalized || "__default__";
}

export function cloneRuntimeState(
  state: ConversationTurnRuntimeState,
): ConversationTurnRuntimeState {
  return {
    ...state,
    toolExecutionLogs: state.toolExecutionLogs.map((entry) => ({ ...entry })),
    currentTurnToolIds: [...state.currentTurnToolIds],
    toolInputById: new Map(state.toolInputById),
    toolNameById: new Map(state.toolNameById),
  };
}

export function snapshotActiveRuntimeState(
  active: ActiveRuntimeRefs,
): ConversationTurnRuntimeState {
  return {
    isGenerating: active.isGenerating.value,
    assistantResponse: active.assistantResponse.value,
    assistantReasoning: active.assistantReasoning.value,
    assistantTokenUsage: active.assistantTokenUsage.value,
    assistantTurnCost: active.assistantTurnCost.value,
    pendingQuestion: active.pendingQuestion.value,
    pendingPermissionRequestId: active.pendingPermissionRequestId.value,
    currentToolStartedAt: active.currentToolStartedAt.value,
    currentToolCalls: active.currentToolCalls.value,
    currentToolDurationMs: active.currentToolDurationMs.value,
    currentContextUsage: cloneContextUsage(active.currentContextUsage.value),
    currentContextTokens: active.currentContextTokens.value,
    currentInputTokens: active.currentInputTokens.value,
    currentOutputTokens: active.currentOutputTokens.value,
    toolExecutionLogs: active.toolExecutionLogs.value.map((entry) => ({ ...entry })),
    currentTurnToolIds: [...active.currentTurnToolIds.value],
    toolInputById: new Map(active.toolInputById),
    toolNameById: new Map(active.toolNameById),
  };
}

export function applyRuntimeStateToActive(
  state: ConversationTurnRuntimeState,
  active: ActiveRuntimeRefs,
) {
  active.isGenerating.value = state.isGenerating;
  active.assistantResponse.value = state.assistantResponse;
  active.assistantReasoning.value = state.assistantReasoning;
  active.assistantTokenUsage.value = state.assistantTokenUsage;
  active.assistantTurnCost.value = state.assistantTurnCost;
  active.pendingQuestion.value = state.pendingQuestion;
  active.pendingPermissionRequestId.value = state.pendingPermissionRequestId;
  active.currentToolStartedAt.value = state.currentToolStartedAt;
  active.currentToolCalls.value = state.currentToolCalls;
  active.currentToolDurationMs.value = state.currentToolDurationMs;
  active.currentContextUsage.value = cloneContextUsage(state.currentContextUsage);
  active.currentContextTokens.value = state.currentContextTokens;
  active.currentInputTokens.value = state.currentInputTokens;
  active.currentOutputTokens.value = state.currentOutputTokens;
  active.toolExecutionLogs.value = state.toolExecutionLogs.map((entry) => ({ ...entry }));
  active.currentTurnToolIds.value = [...state.currentTurnToolIds];

  active.toolInputById.clear();
  for (const [id, input] of state.toolInputById.entries()) {
    active.toolInputById.set(id, input);
  }

  active.toolNameById.clear();
  for (const [id, name] of state.toolNameById.entries()) {
    active.toolNameById.set(id, name);
  }
}

export function clearActiveRuntimeState(active: ActiveRuntimeRefs) {
  active.isGenerating.value = false;
  active.assistantResponse.value = "";
  active.assistantReasoning.value = "";
  active.assistantTokenUsage.value = undefined;
  active.assistantTurnCost.value = undefined;
  active.pendingQuestion.value = null;
  active.pendingPermissionRequestId.value = null;
  active.currentToolStartedAt.value = null;
  active.currentToolCalls.value = 0;
  active.currentToolDurationMs.value = 0;
  active.currentContextUsage.value = undefined;
  active.currentContextTokens.value = 0;
  active.currentInputTokens.value = 0;
  active.currentOutputTokens.value = 0;
  active.toolExecutionLogs.value = [];
  active.currentTurnToolIds.value = [];
  active.toolInputById.clear();
  active.toolNameById.clear();
}

export function cleanupRuntimeStateIfIdle(
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>,
  conversationId: string,
) {
  const key = normalizeConversationId(conversationId);
  const state = runtimeStateByConversation.get(key);
  if (!state) {
    return;
  }

  const hasRenderableResponse = state.assistantResponse.trim().length > 0;
  const hasReasoning = state.assistantReasoning.trim().length > 0;
  const hasPendingPrompt = !!state.pendingPermissionRequestId || !!state.pendingQuestion;
  const hasRunningTool = state.toolExecutionLogs.some((entry) => entry.status === "running");
  if (!state.isGenerating && !hasRenderableResponse && !hasReasoning && !hasPendingPrompt && !hasRunningTool) {
    runtimeStateByConversation.delete(key);
  }
}

export function stashRuntimeState(
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>,
  conversationId: string,
  active: ActiveRuntimeRefs,
) {
  const key = normalizeConversationId(conversationId);
  runtimeStateByConversation.set(key, snapshotActiveRuntimeState(active));
  cleanupRuntimeStateIfIdle(runtimeStateByConversation, key);
}

export function restoreRuntimeState(
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>,
  conversationId: string,
  active: ActiveRuntimeRefs,
): boolean {
  const key = normalizeConversationId(conversationId);
  const state = runtimeStateByConversation.get(key);
  if (!state) {
    clearActiveRuntimeState(active);
    return false;
  }

  applyRuntimeStateToActive(cloneRuntimeState(state), active);
  cleanupRuntimeStateIfIdle(runtimeStateByConversation, key);
  return true;
}

export function ensureRuntimeState(
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>,
  conversationId: string,
): ConversationTurnRuntimeState {
  const key = normalizeConversationId(conversationId);
  const existing = runtimeStateByConversation.get(key);
  if (existing) {
    return existing;
  }

  const created = createEmptyRuntimeState();
  runtimeStateByConversation.set(key, created);
  return created;
}

export function hasAnyGeneratingConversations(
  isGenerating: Ref<boolean>,
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>,
): boolean {
  if (isGenerating.value) {
    return true;
  }

  for (const state of runtimeStateByConversation.values()) {
    if (state.isGenerating) {
      return true;
    }
  }
  return false;
}

export function isSpecificConversationGenerating(
  activeConversationId: Ref<string>,
  isGenerating: Ref<boolean>,
  runtimeStateByConversation: Map<string, ConversationTurnRuntimeState>,
  conversationId: string,
): boolean {
  if (conversationId === activeConversationId.value) {
    return isGenerating.value;
  }

  const state = runtimeStateByConversation.get(normalizeConversationId(conversationId));
  return state?.isGenerating ?? false;
}

export function resetToolTrackingState(active: ActiveRuntimeRefs) {
  active.currentToolStartedAt.value = null;
  active.currentTurnToolIds.value = [];
  active.toolInputById.clear();
  active.toolNameById.clear();
}

export function resetPendingPromptState(active: ActiveRuntimeRefs) {
  active.pendingPermissionRequestId.value = null;
  active.pendingQuestion.value = null;
}

export function resetTurnRuntimeState(active: ActiveRuntimeRefs) {
  resetToolTrackingState(active);
  resetPendingPromptState(active);
}
