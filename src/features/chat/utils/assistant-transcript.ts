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
 * 核心合并逻辑：按正文（text）为边界分组。
 * 只有遇见正文才分开；在同一个无正文区间内：
 * - 所有的思考块（reasoning）合并为同一个思考块，思考内容追加在一起；
 * - 所有的工具块（tools）合并为同一个工具块，工具 ID 收集在一起；
 * - 思考与工具互不混合，保持各自原本的卡片类型与独立性。
 */
export function mergeSegmentsByTextBoundary(
  segments: AssistantTranscriptSegment[],
): AssistantTranscriptSegment[] {
  const result: AssistantTranscriptSegment[] = [];

  let currentReasoning: Extract<AssistantTranscriptSegment, { type: "reasoning" }> | null = null;
  let currentTools: Extract<AssistantTranscriptSegment, { type: "tools" }> | null = null;

  function flushSpan() {
    // 思考块始终放在工具前面
    if (currentReasoning) {
      result.push(currentReasoning);
    }
    if (currentTools) {
      result.push(currentTools);
    }
    currentReasoning = null;
    currentTools = null;
  }

  for (const seg of segments) {
    if (seg.type === "text") {
      flushSpan();
      const last = result[result.length - 1];
      if (last && last.type === "text") {
        last.text = last.text ? `${last.text}\n\n${seg.text}` : seg.text;
      } else {
        result.push({ type: "text", text: seg.text });
      }
    } else if (seg.type === "reasoning") {
      if (!currentReasoning) {
        currentReasoning = { type: "reasoning", text: seg.text };
      } else {
        currentReasoning.text = currentReasoning.text
          ? `${currentReasoning.text}\n\n${seg.text}`
          : seg.text;
      }
    } else if (seg.type === "tools") {
      if (!currentTools) {
        currentTools = { type: "tools", toolIds: [...seg.toolIds] };
      } else {
        for (const id of seg.toolIds) {
          if (!currentTools.toolIds.includes(id)) {
            currentTools.toolIds.push(id);
          }
        }
      }
    }
  }

  flushSpan();
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
  // 按正文划分区间：未被正文分隔的思考追加在思考块，工具追加在工具块
  const next = mergeSegmentsByTextBoundary(filtered);
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
