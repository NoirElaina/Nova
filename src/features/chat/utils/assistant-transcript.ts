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

/**
 * 就地追加正文。同类型末段直接改 text，避免每 token 深拷贝整表。
 * 结构变化（新开 segment）时 push；调用方若用 shallowRef 需在结构变化时触发替换。
 */
export function appendTranscriptText(
  segments: AssistantTranscriptSegment[],
  text: string,
): AssistantTranscriptSegment[] {
  if (!text) {
    return segments;
  }

  const last = segments[segments.length - 1];
  if (last?.type === "text") {
    last.text += text;
    return segments;
  }

  segments.push({ type: "text", text });
  return segments;
}

export function appendTranscriptReasoning(
  segments: AssistantTranscriptSegment[],
  text: string,
): AssistantTranscriptSegment[] {
  if (!text) {
    return segments;
  }

  const last = segments[segments.length - 1];
  if (last?.type === "reasoning") {
    last.text += text;
    return segments;
  }

  segments.push({ type: "reasoning", text });
  return segments;
}

export function appendTranscriptTool(
  segments: AssistantTranscriptSegment[],
  toolId: string,
): AssistantTranscriptSegment[] {
  if (!toolId) {
    return segments;
  }

  const last = segments[segments.length - 1];
  if (last?.type === "tools") {
    if (!last.toolIds.includes(toolId)) {
      last.toolIds.push(toolId);
    }
    return segments;
  }

  segments.push({ type: "tools", toolIds: [toolId] });
  return segments;
}

function hasDisplayableSegment(segment: AssistantTranscriptSegment): boolean {
  if (segment.type === "tools") {
    return segment.toolIds.length > 0;
  }
  return segment.text.trim().length > 0;
}

/**
 * 合并直接相邻且同类型的段（例如两个连续的 reasoning、连续的 text、或并发调用的相邻 tools）。
 * 绝不跨段强行将相隔的 tools 和 reasoning 打包，严格保持与流式运行期一致的真实时序。
 */
function mergeAdjacentSegments(
  segments: AssistantTranscriptSegment[],
): AssistantTranscriptSegment[] {
  const result: AssistantTranscriptSegment[] = [];
  for (const seg of segments) {
    const last = result[result.length - 1];
    if (last && last.type === seg.type) {
      if (last.type === "reasoning" && seg.type === "reasoning") {
        last.text = last.text ? `${last.text}\n\n${seg.text}` : seg.text;
      } else if (last.type === "text" && seg.type === "text") {
        last.text = last.text ? `${last.text}\n\n${seg.text}` : seg.text;
      } else if (last.type === "tools" && seg.type === "tools") {
        for (const id of seg.toolIds) {
          if (!last.toolIds.includes(id)) {
            last.toolIds.push(id);
          }
        }
      }
    } else {
      result.push(
        seg.type === "tools"
          ? { type: "tools", toolIds: [...seg.toolIds] }
          : { ...seg }
      );
    }
  }
  return result;
}

export function buildAssistantTranscriptSegments(
  segments: AssistantTranscriptSegment[] | undefined,
  options: {
    reasoning?: string;
    text?: string;
  } = {},
): AssistantTranscriptSegment[] {
  const filtered = cloneTranscriptSegments(segments).filter(hasDisplayableSegment);
  // 合并直接相邻同类型段，保持与流式期间一致的时间线结构
  const next = mergeAdjacentSegments(filtered);
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
  const byId = new Map<string, ToolExecutionEntry>();
  for (const entry of entries) {
    const existing = byId.get(entry.id);
    if (!existing || existing.status === "running" || entry.status !== "running") {
      byId.set(entry.id, entry);
    }
  }
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
