import type {
  AssistantTranscriptSegment,
  NeedsUserInputPayload,
  ContextCompactSummary,
  ContextUsage,
  ToolExecutionEntry,
  TurnCost,
} from "../../../lib/chat-types";

export type MainView = "chat" | "hooks" | "agent" | "agentMarket" | "schedule" | "settings";
export type LiveTurnStage = "processing" | "compacting";

export type BackendErrorEvent = {
  source?: string;
  code?: string;
  title?: string;
  message?: string;
  stage?: string | null;
};

export type ScheduledTaskTriggerEvent = {
  id: string;
  conversationId?: string;
  cron: string;
  prompt: string;
  recurring: boolean;
  durable: boolean;
  createdAt?: string;
  triggeredAt?: string;
};

export type ChatScreenHandle = {
  scrollToBottom: () => void;
  scrollLastUserMessageToTop: () => void;
  scrollLastUserMessageToBottom: () => void;
  scrollLiveAssistantIntoView: () => void;
};

export type ModelTextBlock = {
  type: "text";
  text: string;
};

export type ModelImageBlock = {
  type: "image";
  source: {
    type: "base64";
    media_type: string;
    data: string;
  };
};

export type ModelMessage = {
  role: "user" | "assistant";
  content: string | Array<ModelTextBlock | ModelImageBlock>;
};

export const SCHEDULED_CONVERSATION_TITLE_PREFIX = "Scheduled [";

export type ConversationTurnRuntimeState = {
  isGenerating: boolean;
  currentStage: LiveTurnStage;
  /** 本轮发起时的 agent 模式（后台会话权限自动放行依赖此字段） */
  agentMode: import("../../../lib/chat-types").AgentMode;
  assistantResponse: string;
  assistantReasoning: string;
  assistantSegments: AssistantTranscriptSegment[];
  assistantTokenUsage?: number;
  assistantTurnCost?: TurnCost;
  pendingQuestion: NeedsUserInputPayload | null;
  pendingPermissionRequestId: string | null;
  /** 本轮开始时间（ms epoch）：发送时打点，恢复时取后端 live_turns.startedAt */
  currentTurnStartedAt: number | null;
  currentToolStartedAt: number | null;
  currentToolCalls: number;
  currentToolDurationMs: number;
  currentContextUsage?: ContextUsage;
  currentContextCompacts: ContextCompactSummary[];
  currentContextTokens: number;
  currentInputTokens: number;
  currentOutputTokens: number;
  currentTurnId: string | null;
  toolExecutionLogs: ToolExecutionEntry[];
  currentTurnToolIds: string[];
  toolInputById: Map<string, string>;
  toolNameById: Map<string, string>;
};
