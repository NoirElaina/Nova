import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AgentDomainEvent } from "./agent-events";

export interface ModernAgentCallbacks {
  onTurnStarted?: (turnId: string, conversationId: string) => void;
  onStateChanged?: (turnId: string, state: string) => void;
  onThinkingDelta?: (delta: string, turnId?: string) => void;
  onTextDelta?: (delta: string, turnId?: string) => void;
  onToolRequested?: (callId: string, toolName: string, args: Record<string, unknown>, turnId?: string) => void;
  onToolCompleted?: (callId: string, toolName: string, output: string, isError: boolean, durationMs: number, turnId?: string) => void;
  onVerificationStarted?: (target: string) => void;
  onVerificationCompleted?: (target: string, passed: boolean, feedback?: string) => void;
  onTokenUsage?: (usage: { input: number; output: number; cacheRead: number; cacheCreate: number }, turnId?: string) => void;
  onPermissionRequested?: (turnId: string, requestId: string, toolName: string, payload: string) => void;
  onTurnFinished?: (turnId: string, stopReason: string) => void;
  onError?: (error: string, turnId?: string) => void;
}

/**
 * 监听来自 Rust 核心引擎的领域事件
 * 前端彻底摆脱任何状态组装与重推断，纯响应式渲染
 */
export async function setupAgentEventListener(
  callbacks: ModernAgentCallbacks
): Promise<UnlistenFn> {
  return await listen<AgentDomainEvent>("agent-event", (event) => {
    const data = event.payload;

    switch (data.kind) {
      case "turn_started":
        callbacks.onTurnStarted?.(data.payload.turn_id, data.payload.conversation_id);
        break;

      case "state_changed":
        callbacks.onStateChanged?.(data.payload.turn_id, data.payload.state);
        break;

      case "thinking_delta":
        callbacks.onThinkingDelta?.(data.payload.delta, data.payload.turn_id);
        break;

      case "text_delta":
        callbacks.onTextDelta?.(data.payload.delta, data.payload.turn_id);
        break;

      case "tool_call_requested":
        callbacks.onToolRequested?.(
          data.payload.call_id,
          data.payload.tool_name,
          data.payload.arguments,
          data.payload.turn_id
        );
        break;

      case "tool_call_completed":
        callbacks.onToolCompleted?.(
          data.payload.call_id,
          data.payload.tool_name,
          data.payload.output,
          data.payload.is_error,
          data.payload.duration_ms,
          data.payload.turn_id
        );
        break;

      case "verification_started":
        callbacks.onVerificationStarted?.(data.payload.target);
        break;

      case "verification_completed":
        callbacks.onVerificationCompleted?.(
          data.payload.target,
          data.payload.passed,
          data.payload.feedback
        );
        break;

      case "token_usage_update":
        callbacks.onTokenUsage?.({
          input: data.payload.input_tokens,
          output: data.payload.output_tokens,
          cacheRead: data.payload.cache_read_tokens,
          cacheCreate: data.payload.cache_creation_tokens,
        }, data.payload.turn_id);
        break;

      case "permission_requested":
        callbacks.onPermissionRequested?.(
          data.payload.turn_id,
          data.payload.request_id,
          data.payload.tool_name,
          data.payload.payload
        );
        break;

      case "turn_finished":
        callbacks.onTurnFinished?.(data.payload.turn_id, data.payload.stop_reason);
        break;

      case "turn_error":
        callbacks.onError?.(data.payload.error, data.payload.turn_id);
        break;
    }
  });
}

