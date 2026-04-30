import type { ToolExecutionEntry } from "../../../lib/chat-types";
import type { ConversationTurnRuntimeState } from "./chat-controller-types";

type PersistToolExecutionLog = (
  entry: ToolExecutionEntry,
  conversationId?: string,
) => void;

export function findToolExecutionIndexById(
  entries: ToolExecutionEntry[],
  toolId: string,
): number {
  return entries.findIndex((entry) => entry.id === toolId);
}

export function latestRunningToolExecutionIdByName(
  entries: ToolExecutionEntry[],
  toolName: string,
): string | null {
  for (let i = entries.length - 1; i >= 0; i -= 1) {
    const entry = entries[i];
    if (entry.toolName === toolName && entry.status === "running") {
      return entry.id;
    }
  }
  return null;
}

export function startToolExecutionTraceInState(
  state: ConversationTurnRuntimeState,
  toolId: string,
  toolName: string,
) {
  const idx = findToolExecutionIndexById(state.toolExecutionLogs, toolId);
  if (idx >= 0) {
    state.toolExecutionLogs[idx] = {
      ...state.toolExecutionLogs[idx],
      toolName,
      status: "running",
      startedAt: Date.now(),
      finishedAt: undefined,
    };
    return;
  }

  state.toolExecutionLogs.push({
    id: toolId,
    toolName,
    input: "",
    result: "",
    status: "running",
    startedAt: Date.now(),
    finishedAt: undefined,
  });
}

export function appendToolExecutionInputInState(
  state: ConversationTurnRuntimeState,
  toolId: string,
  inputDelta: string,
) {
  const idx = findToolExecutionIndexById(state.toolExecutionLogs, toolId);
  if (idx < 0) {
    return;
  }

  const entry = state.toolExecutionLogs[idx];
  state.toolExecutionLogs[idx] = {
    ...entry,
    input: `${entry.input}${inputDelta}`,
  };
}

export function completeToolExecutionTraceInState(
  conversationId: string,
  state: ConversationTurnRuntimeState,
  toolId: string | null,
  toolName: string,
  result: string,
  status: ToolExecutionEntry["status"],
  persistToolExecutionLog: PersistToolExecutionLog,
  inputFallback?: string,
) {
  const resolvedId =
    toolId || latestRunningToolExecutionIdByName(state.toolExecutionLogs, toolName);
  if (!resolvedId) {
    return;
  }

  const idx = findToolExecutionIndexById(state.toolExecutionLogs, resolvedId);
  if (idx < 0) {
    return;
  }

  const entry = state.toolExecutionLogs[idx];
  const normalizedFallback = (inputFallback ?? "").trim();
  const resolvedInput = entry.input.trim().length > 0 ? entry.input : normalizedFallback;
  const updatedEntry: ToolExecutionEntry = {
    ...entry,
    toolName,
    input: resolvedInput,
    result,
    status,
    finishedAt: Date.now(),
  };

  state.toolExecutionLogs[idx] = updatedEntry;
  persistToolExecutionLog(updatedEntry, conversationId);
}

export function markRunningToolExecutionsInState(
  conversationId: string,
  state: ConversationTurnRuntimeState,
  status: "completed" | "error" | "cancelled",
  persistToolExecutionLog: PersistToolExecutionLog,
) {
  const now = Date.now();
  for (let i = 0; i < state.toolExecutionLogs.length; i += 1) {
    const entry = state.toolExecutionLogs[i];
    if (entry.status !== "running") {
      continue;
    }

    const updatedEntry: ToolExecutionEntry = {
      ...entry,
      status,
      finishedAt: now,
    };
    state.toolExecutionLogs[i] = updatedEntry;
    persistToolExecutionLog(updatedEntry, conversationId);
  }
}
