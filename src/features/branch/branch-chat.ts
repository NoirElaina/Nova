// 分支对话前端状态：内存态单例，不持久化（刷新/重启即丢弃）。
// 生命周期：openBranch 创建会话 → sendBranchMessage 逐轮问答（历史全量上传，
// 后端无状态）→ closeBranch 丢弃。stream 增量与生命周期状态经 `branch-event`
// 通道推送，与主会话 chat-stream 完全隔离。

import { computed, reactive, ref } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import {
  appendConversationMessage,
  createConversation,
} from '../chat/services/chat-api';
import { emitToast } from '../../lib/toast';
import { getRawErrorText } from '../../lib/error-display';

export type BranchRole = 'user' | 'assistant';

export interface BranchMessage {
  role: BranchRole;
  content: string;
  reasoning?: string;
  createdAt: number;
}

export type BranchPhase = 'idle' | 'running';

export interface BranchSession {
  branchId: string;
  parentConversationId: string;
  /** 开分支时在主对话中选中的原文 */
  quotedText: string;
  messages: BranchMessage[];
  /** 流式进行中的正文/思考增量，done 时并入 messages */
  streamingText: string;
  streamingReasoning: string;
  phase: BranchPhase;
  error?: string;
  createdAt: number;
  /** 已转存为正式会话后标记，避免重复导出 */
  exported?: boolean;
}

interface BranchStreamEventPayload {
  type?: string;
  text?: string | null;
}

interface BranchEventPayload {
  kind: 'status' | 'stream';
  parentConversationId?: string;
  branchId?: string;
  phase?: string;
  detail?: string | null;
  event?: BranchStreamEventPayload;
}

const sessions = reactive(new Map<string, BranchSession>());

/** 侧边栏开关与当前激活分支（跨组件共享）。 */
export const sidebarOpen = ref(false);
export const activeBranchId = ref<string | null>(null);

let listenerReady = false;
let initPromise: Promise<void> | null = null;

function finalizeStream(session: BranchSession) {
  const text = session.streamingText.trim();
  if (text) {
    session.messages.push({
      role: 'assistant',
      content: session.streamingText,
      reasoning: session.streamingReasoning.trim() || undefined,
      createdAt: Date.now(),
    });
  }
  session.streamingText = '';
  session.streamingReasoning = '';
  session.phase = 'idle';
}

function handleBranchEvent(payload: BranchEventPayload) {
  const branchId = (payload.branchId ?? '').trim();
  if (!branchId) return;
  const session = sessions.get(branchId);
  if (!session) return;

  if (payload.kind === 'stream' && payload.event) {
    const event = payload.event;
    if (event.type === 'text' && event.text) {
      session.streamingText += event.text;
    } else if (event.type === 'reasoning' && event.text) {
      session.streamingReasoning += event.text;
    }
    // usage / stop 等事件不进 UI：收尾以 status(done/error) 为准。
    return;
  }

  if (payload.kind === 'status') {
    if (payload.phase === 'start') {
      session.phase = 'running';
      session.error = undefined;
      return;
    }
    if (payload.phase === 'done') {
      finalizeStream(session);
      return;
    }
    if (payload.phase === 'error') {
      // 保留已流出的部分内容，再附上错误，用户看得见上文
      finalizeStream(session);
      session.error = (payload.detail ?? '').trim() || '分支请求失败';
    }
  }
}

export function initBranchEvents(): Promise<void> {
  if (listenerReady) return Promise.resolve();
  if (initPromise) return initPromise;
  initPromise = listen<BranchEventPayload>('branch-event', (event) => {
    handleBranchEvent(event.payload);
  })
    .then(() => {
      listenerReady = true;
    })
    .catch((err) => {
      console.error('Failed to listen branch-event:', err);
      initPromise = null;
    });
  return initPromise;
}

function genBranchId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID().replace(/-/g, '').slice(0, 12);
  }
  return `b${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

/** 开启一个分支：选中原文作为锚点，弹出侧边栏。 */
export function openBranch(parentConversationId: string, quotedText: string): BranchSession {
  const session: BranchSession = {
    branchId: genBranchId(),
    parentConversationId,
    quotedText,
    messages: [],
    streamingText: '',
    streamingReasoning: '',
    phase: 'idle',
    createdAt: Date.now(),
  };
  sessions.set(session.branchId, session);
  activeBranchId.value = session.branchId;
  sidebarOpen.value = true;
  return session;
}

export function branchesFor(parentConversationId: string | null | undefined): BranchSession[] {
  if (!parentConversationId) return [];
  return Array.from(sessions.values())
    .filter((s) => s.parentConversationId === parentConversationId)
    .sort((a, b) => a.createdAt - b.createdAt);
}

export function getBranch(branchId: string | null | undefined): BranchSession | null {
  if (!branchId) return null;
  return sessions.get(branchId) ?? null;
}

export const activeBranch = computed(() => getBranch(activeBranchId.value));

export function setActiveBranch(branchId: string) {
  if (sessions.has(branchId)) {
    activeBranchId.value = branchId;
    sidebarOpen.value = true;
  }
}

/** 关闭并丢弃一个分支；进行中先取消。 */
export function closeBranch(branchId: string) {
  const session = sessions.get(branchId);
  if (!session) return;
  if (session.phase === 'running') {
    void invoke('cancel_branch_message', {
      parentConversationId: session.parentConversationId,
      branchId: session.branchId,
    }).catch(() => false);
  }
  sessions.delete(branchId);
  if (activeBranchId.value === branchId) {
    const siblings = branchesFor(session.parentConversationId);
    activeBranchId.value = siblings.length > 0 ? siblings[siblings.length - 1].branchId : null;
  }
  if (sessions.size === 0) {
    sidebarOpen.value = false;
  }
}

export function closeSidebar() {
  sidebarOpen.value = false;
}

/** 发送一轮分支问答。首轮自动把引用原文包进首条用户消息（仅发给模型，不改动展示）。 */
export async function sendBranchMessage(session: BranchSession, question: string) {
  const text = question.trim();
  if (!text || session.phase === 'running') return;

  session.messages.push({ role: 'user', content: text, createdAt: Date.now() });
  session.phase = 'running';
  session.error = undefined;
  session.streamingText = '';
  session.streamingReasoning = '';

  const payloadMessages = session.messages.map((m, index) => ({
    role: m.role,
    content:
      index === 0 && m.role === 'user'
        ? `我在主对话中读到以下内容：\n\n"""\n${session.quotedText}\n"""\n\n我的问题是：${m.content}`
        : m.content,
  }));

  try {
    await invoke('send_branch_message', {
      parentConversationId: session.parentConversationId,
      branchId: session.branchId,
      messages: payloadMessages,
    });
    // 正常路径：status(done) 事件已收尾；若事件先于此 resolve 到达也没问题（幂等）。
    if (session.phase === 'running') {
      finalizeStream(session);
    }
  } catch (err) {
    if (session.phase === 'running') {
      finalizeStream(session);
      if (!session.error) {
        session.error = getRawErrorText(err) || '分支请求失败';
      }
    }
  }
}

/** 取消当前分支正在进行的回复。 */
export async function cancelBranch(session: BranchSession) {
  if (session.phase !== 'running') return;
  const hit = await invoke<boolean>('cancel_branch_message', {
    parentConversationId: session.parentConversationId,
    branchId: session.branchId,
  }).catch(() => false);
  // 未命中运行中的轮次时不会有 status 事件回推，本地直接收尾
  if (!hit && session.phase === 'running') {
    finalizeStream(session);
  }
}

/** 把分支内容转存为正式会话（唯一的"沉淀"出口），转存后可关闭分支不丢失。 */
export async function exportBranchAsConversation(session: BranchSession) {
  if (session.exported || session.messages.length === 0 || session.phase === 'running') return;
  const titleSeed = session.quotedText.replace(/\s+/g, ' ').trim().slice(0, 24) || '分支问答';
  try {
    const meta = await createConversation(`分支：${titleSeed}`);
    for (const [index, m] of session.messages.entries()) {
      const content =
        index === 0 && m.role === 'user'
          ? `我在主对话中读到以下内容：\n\n> ${session.quotedText.split('\n').join('\n> ')}\n\n我的问题是：${m.content}`
          : m.content;
      await appendConversationMessage(meta.id, { role: m.role, content });
    }
    session.exported = true;
    emitToast({ message: '已保存为新会话，可在左侧会话列表查看', variant: 'success' });
  } catch (err) {
    emitToast({
      message: `保存分支失败：${getRawErrorText(err) || '未知错误'}`,
      variant: 'error',
    });
  }
}
