import type {
  AssistantTranscriptSegment,
  ChatMessage,
  ToolExecutionEntry,
  ToolTurnSummary,
} from "../../../lib/chat-types";
import { buildToolTurnSummary } from "./tool-activity-summary";

const EMPTY_ASSISTANT_CONTENT = "（本轮没有返回可显示的文本内容）";

export function cloneTranscriptSegments(
  segments: AssistantTranscriptSegment[] | undefined,
): AssistantTranscriptSegment[] {
  return (segments ?? []).map((segment) =>
    segment.type === "tools"
      ? { type: "tools", toolIds: [...segment.toolIds] }
      : { ...segment },
  );
}

export function appendTranscriptText(
  segments: AssistantTranscriptSegment[],
  text: string,
): AssistantTranscriptSegment[] {
  if (!text) {
    return segments;
  }

  const next = cloneTranscriptSegments(segments);
  const last = next[next.length - 1];
  if (last?.type === "text") {
    last.text += text;
    return next;
  }

  next.push({ type: "text", text });
  return next;
}

export function appendTranscriptReasoning(
  segments: AssistantTranscriptSegment[],
  text: string,
): AssistantTranscriptSegment[] {
  if (!text) {
    return segments;
  }

  const next = cloneTranscriptSegments(segments);
  const last = next[next.length - 1];
  if (last?.type === "reasoning") {
    last.text += text;
    return next;
  }

  next.push({ type: "reasoning", text });
  return next;
}

export function appendTranscriptTool(
  segments: AssistantTranscriptSegment[],
  toolId: string,
): AssistantTranscriptSegment[] {
  if (!toolId) {
    return segments;
  }

  const next = cloneTranscriptSegments(segments);
  const last = next[next.length - 1];
  if (last?.type === "tools") {
    if (!last.toolIds.includes(toolId)) {
      last.toolIds.push(toolId);
    }
    return next;
  }

  next.push({ type: "tools", toolIds: [toolId] });
  return next;
}

function hasDisplayableSegment(segment: AssistantTranscriptSegment): boolean {
  if (segment.type === "tools") {
    return segment.toolIds.length > 0;
  }
  return segment.text.trim().length > 0;
}

export function buildAssistantTranscriptSegments(
  segments: AssistantTranscriptSegment[] | undefined,
  options: {
    reasoning?: string;
    text?: string;
  } = {},
): AssistantTranscriptSegment[] {
  const next = cloneTranscriptSegments(segments).filter(hasDisplayableSegment);
  const reasoning = options.reasoning?.trim();
  const text = options.text?.trim();

  if (reasoning && !next.some((segment) => segment.type === "reasoning")) {
    next.unshift({ type: "reasoning", text: reasoning });
  }

  if (text && !next.some((segment) => segment.type === "text")) {
    next.push({ type: "text", text });
  }

  return next;
}

export function normalizeAssistantTranscript(message: ChatMessage): AssistantTranscriptSegment[] {
  const stored = message.transcriptSegments ?? message.cost?.transcriptSegments;
  const content = message.content.trim();
  const text =
    content === EMPTY_ASSISTANT_CONTENT && message.reasoning?.trim()
      ? undefined
      : message.content;

  return buildAssistantTranscriptSegments(stored, {
    reasoning: message.reasoning,
    text,
  });
}

export function buildToolSummaryForSegment(
  segment: Extract<AssistantTranscriptSegment, { type: "tools" }>,
  entries: ToolExecutionEntry[],
  snapshot?: ToolTurnSummary,
): ToolTurnSummary | undefined {
  const byId = new Map(entries.map((entry) => [entry.id, entry]));
  const liveEntries = segment.toolIds
    .map((id) => byId.get(id))
    .filter((entry): entry is ToolExecutionEntry => !!entry);

  if (liveEntries.length > 0) {
    return buildToolTurnSummary(liveEntries);
  }

  const snapshotEntries = snapshot?.entries
    .filter((entry) => segment.toolIds.includes(entry.id))
    .map((entry) => ({ ...entry }));

  return snapshotEntries && snapshotEntries.length > 0
    ? buildToolTurnSummary(snapshotEntries)
    : undefined;
}
