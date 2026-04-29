export interface TurnCost {
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  toolDurationMs: number;
}

export type AgentMode = "agent" | "plan" | "auto";

export type AttachmentKind = "document" | "image";

export interface ChatAttachment {
  sourceName: string;
  mimeType?: string;
  size?: number;
  kind?: AttachmentKind;
  mediaType?: string;
  data?: string;
}

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
  reasoning?: string;
  attachments?: ChatAttachment[];
  tokenUsage?: number;
  cost?: TurnCost;
}

export interface UploadedRagFile extends ChatAttachment {
  kind: "document";
  content: string;
  size: number;
}

export interface UploadedImageFile extends ChatAttachment {
  kind: "image";
  mediaType: string;
  data: string;
  size: number;
}

export type PendingUploadFile = UploadedRagFile | UploadedImageFile;

export interface PersistedMessage {
  role: string;
  content: string;
  reasoning?: string;
  attachments?: ChatAttachment[];
  tokenUsage?: number;
  cost?: TurnCost;
}

export interface ConversationMemory {
  summary: string;
  keyFacts: string[];
  updatedAt: number;
}

export interface ConversationMeta {
  id: string;
  title: string;
  updatedAt: number;
}

export interface ScheduledTask {
  id: string;
  cron: string;
  prompt: string;
  conversationId?: string;
  recurring: boolean;
  durable: boolean;
  createdAt: string;
}

export interface ChatMessageEvent {
  type: string;
  conversation_id?: string;
  text?: string;
  tool_use_id?: string;
  tool_use_name?: string;
  tool_use_input?: string;
  tool_result?: string;
  token_usage?: number;
  stop_reason?: string;
  turn_state?: string;
}

export interface ToolExecutionEntry {
  id: string;
  toolName: string;
  input: string;
  result: string;
  status: "running" | "completed" | "error" | "cancelled";
  startedAt: number;
  finishedAt?: number;
}

export interface FlowNodeEntry {
  nodeId: string;
  label: string;
  status: "running" | "completed" | "skipped" | "error";
  detail?: string;
  conversationId?: string;
  timestamp: number;
}

export interface AskUserOption {
  label: string;
  description: string;
  value?: string;
  preview?: string;
}

export interface AskUserQuestionItem {
  question: string;
  header: string;
  options: AskUserOption[];
  multi_select?: boolean;
}

export interface NeedsUserInputPayload {
  type?: string;
  context?: string;
  allow_freeform?: boolean;
  questions?: AskUserQuestionItem[];
}

export interface AskUserAnswerSubmission {
  answers: Record<string, string | string[]>;
  freeform?: string;
}

export interface PlanModeChangePayload {
  type?: string;
  mode?: string;
  goal?: string;
  summary?: string;
  message?: string;
}
