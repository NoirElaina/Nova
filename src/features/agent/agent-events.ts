export type AgentDomainEvent =
  | {
      kind: "turn_started";
      payload: {
        turn_id: string;
        conversation_id: string;
        timestamp: number;
      };
    }
  | {
      kind: "thinking_delta";
      payload: {
        turn_id: string;
        delta: string;
      };
    }
  | {
      kind: "text_delta";
      payload: {
        turn_id: string;
        delta: string;
      };
    }
  | {
      kind: "tool_call_requested";
      payload: {
        turn_id: string;
        call_id: string;
        tool_name: string;
        arguments: Record<string, unknown>;
      };
    }
  | {
      kind: "tool_call_completed";
      payload: {
        turn_id: string;
        call_id: string;
        tool_name: string;
        is_error: boolean;
        output: string;
        duration_ms: number;
      };
    }
  | {
      kind: "verification_started";
      payload: {
        turn_id: string;
        target: string;
      };
    }
  | {
      kind: "verification_completed";
      payload: {
        turn_id: string;
        target: string;
        passed: boolean;
        feedback?: string;
      };
    }
  | {
      kind: "token_usage_update";
      payload: {
        turn_id: string;
        input_tokens: number;
        output_tokens: number;
        cache_read_tokens: number;
        cache_creation_tokens: number;
      };
    }
  | {
      kind: "turn_finished";
      payload: {
        turn_id: string;
        stop_reason: string;
        total_tokens: number;
      };
    }
  | {
      kind: "turn_error";
      payload: {
        turn_id: string;
        error: string;
      };
    };

