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
 * 将相邻的 reasoning/tools 块（没有被 text 正文分隔的）合并成一组。
 *
 * 背景：当 AI 进行多轮 thinking → tool → thinking → tool 循环而没有输出正文时，
 * 流式追加会产生交替的 reasoning/tools segments。这里将它们"按正文分组"合并，
 * 使得每个"正文块"之前的所有思考和工具合并为最多一个 reasoning + 一个 tools。
 */
function mergeAdjacentNonTextSegments(
  segments: AssistantTranscriptSegment[],
): AssistantTranscriptSegment[] {
  const result: AssistantTranscriptSegment[] = [];

  // 累计当前"组"里尚未遇到正文的 reasoning 文本和 toolIds
  let pendingReasoningText = "";
  const pendingToolIds: string[] = [];

  function flushPending() {
    if (pendingReasoningText) {
      result.push({ type: "reasoning", text: pendingReasoningText });
      pendingReasoningText = "";
    }
    if (pendingToolIds.length > 0) {
      result.push({ type: "tools", toolIds: [...pendingToolIds] });
      pendingToolIds.length = 0;
    }
  }

  for (const seg of segments) {
    if (seg.type === "text") {
      // 遇到正文：先把积累的 reasoning/tools flush 出去，再输出正文
      flushPending();
      result.push(seg);
    } else if (seg.type === "reasoning") {
      pendingReasoningText = pendingReasoningText
        ? pendingReasoningText + "\n\n" + seg.text
        : seg.text;
    } else if (seg.type === "tools") {
      for (const id of seg.toolIds) {
        if (!pendingToolIds.includes(id)) {
          pendingToolIds.push(id);
        }
      }
    }
  }

  // 末尾可能还有未 flush 的 reasoning/tools
  flushPending();

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
  // 先合并相邻非正文块，再补充 fallback reasoning/text
  const next = mergeAdjacentNonTextSegments(filtered);
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
