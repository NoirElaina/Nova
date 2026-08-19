// 子代理实时状态：监听后端 `subagent-event`，维护每个子代理的运行日志。
// 生命周期事件（kind=status）建立/更新条目；流式事件（kind=stream）把包装的
// ChatMessageEvent 增量转成侧边栏日志行。状态为模块级单例，多组件共享。

import { reactive } from 'vue';
import { listen } from '@tauri-apps/api/event';

export type SubagentLine = {
  kind: 'text' | 'reasoning' | 'tool' | 'tool-result';
  text: string;
  toolName?: string;
  isError?: boolean;
};

export type SubagentEntry = {
  subId: string;
  parentConversationId: string;
  task: string;
  phase: 'running' | 'done' | 'error';
  startedAt: number;
  elapsedMs?: number;
  detail?: string;
  reportPreview?: string;
  lines: SubagentLine[];
};

interface ChatStreamEventPayload {
  type: string;
  text?: string | null;
  tool_use_name?: string | null;
  tool_result?: string | null;
  tool_is_error?: boolean | null;
}

interface SubagentEventPayload {
  kind: 'status' | 'stream';
  parentConversationId?: string;
  subId?: string;
  phase?: string;
  task?: string;
  detail?: string | null;
  reportPreview?: string | null;
  elapsedMs?: number;
  event?: ChatStreamEventPayload;
}

const entries = reactive(new Map<string, SubagentEntry>());

let listenerReady = false;
let initPromise: Promise<void> | null = null;

function appendStreamLine(entry: SubagentEntry, event: ChatStreamEventPayload) {
  switch (event.type) {
    case 'text': {
      if (!event.text) return;
      const last = entry.lines[entry.lines.length - 1];
      if (last && last.kind === 'text') {
        last.text += event.text;
      } else {
        entry.lines.push({ kind: 'text', text: event.text });
      }
      break;
    }
    case 'reasoning': {
      if (!event.text) return;
      const last = entry.lines[entry.lines.length - 1];
      if (last && last.kind === 'reasoning') {
        last.text += event.text;
      } else {
        entry.lines.push({ kind: 'reasoning', text: event.text });
      }
      break;
    }
    case 'tool-use-start':
      entry.lines.push({
        kind: 'tool',
        text: '',
        toolName: event.tool_use_name ?? 'tool',
      });
      break;
    case 'tool-result': {
      const result = (event.tool_result ?? '').trim();
      entry.lines.push({
        kind: 'tool-result',
        text: result.length > 600 ? `${result.slice(0, 600)}…` : result || '(空结果)',
        toolName: event.tool_use_name ?? undefined,
        isError: event.tool_is_error === true,
      });
      break;
    }
    default:
      // tool-json-delta / tool-executing / stop 等噪声事件不进日志。
      break;
  }
}

function handleSubagentEvent(payload: SubagentEventPayload) {
  const subId = (payload.subId ?? '').trim();
  if (!subId) return;

  if (payload.kind === 'status') {
    const existing = entries.get(subId);
    const phase =
      payload.phase === 'start' ? 'running' : payload.phase === 'error' ? 'error' : 'done';
    if (existing) {
      existing.phase = phase;
      if (typeof payload.elapsedMs === 'number') existing.elapsedMs = payload.elapsedMs;
      if (payload.detail) existing.detail = payload.detail;
      if (payload.reportPreview) existing.reportPreview = payload.reportPreview;
    } else {
      entries.set(subId, {
        subId,
        parentConversationId: payload.parentConversationId ?? '',
        task: payload.task ?? '',
        phase,
        startedAt: Date.now(),
        elapsedMs: payload.elapsedMs,
        detail: payload.detail ?? undefined,
        reportPreview: payload.reportPreview ?? undefined,
        lines: [],
      });
    }
    return;
  }

  if (payload.kind === 'stream' && payload.event) {
    const entry = entries.get(subId);
    if (entry) {
      appendStreamLine(entry, payload.event);
    }
  }
}

export function initSubagentEvents(): Promise<void> {
  if (listenerReady) return Promise.resolve();
  if (initPromise) return initPromise;
  initPromise = listen<SubagentEventPayload>('subagent-event', (event) => {
    handleSubagentEvent(event.payload);
  })
    .then(() => {
      listenerReady = true;
    })
    .catch((err) => {
      console.error('Failed to listen subagent-event:', err);
      initPromise = null;
    });
  return initPromise;
}

export function subagentsFor(parentConversationId: string | null | undefined): SubagentEntry[] {
  if (!parentConversationId) return [];
  return Array.from(entries.values())
    .filter((entry) => entry.parentConversationId === parentConversationId)
    .sort((a, b) => b.startedAt - a.startedAt);
}

export function clearSubagents(parentConversationId: string | null | undefined) {
  if (!parentConversationId) return;
  for (const [subId, entry] of entries) {
    if (entry.parentConversationId === parentConversationId) {
      entries.delete(subId);
    }
  }
}
