import type {
  AskUserAnswerSubmission,
  NeedsUserInputPayload,
  PlanModeChangePayload,
} from "./chat-types";

export type PermissionActionName = "allow_once" | "allow_session" | "deny_session";

function detectPermissionActionFromText(text: string): PermissionActionName | null {
  const normalized = text.trim().toLowerCase();
  if (!normalized) return null;

  if (normalized === "allow_once") {
    return "allow_once";
  }
  if (normalized === "allow_session") {
    return "allow_session";
  }
  if (normalized === "deny_session") {
    return "deny_session";
  }
  return null;
}

export function extractPermissionActionFromAnswers(
  payload: AskUserAnswerSubmission,
): PermissionActionName | null {
  const answerCandidates = payload.answerItems?.map((item) => item.answer) ?? Object.values(payload.answers);

  for (const answer of answerCandidates) {
    if (Array.isArray(answer)) {
      for (const candidate of answer) {
        const action = detectPermissionActionFromText(candidate);
        if (action) {
          return action;
        }
      }
      continue;
    }

    const action = detectPermissionActionFromText(answer);
    if (action) {
      return action;
    }
  }

  return null;
}

export function buildPendingQuestionReply(
  payload: AskUserAnswerSubmission | null,
  source: "submit" | "skip",
): string {
  if (source === "skip" || !payload) {
    return "用户跳过了澄清问题，请基于当前上下文继续；如果仍有缺失，请明确说明你做了哪些假设。";
  }

  const lines: string[] = ["用户补充了以下澄清信息："];
  const answerItems =
    payload.answerItems?.map((item) => ({
      label: item.question.trim() || item.header.trim() || item.key,
      answer: item.answer,
    })) ??
    Object.entries(payload.answers).map(([question, answer]) => ({
      label: question,
      answer,
    }));

  for (const item of answerItems) {
    const answerText = Array.isArray(item.answer) ? item.answer.join("、") : item.answer;
    if (answerText.trim()) {
      lines.push(`- ${item.label}：${answerText}`);
    }
  }

  return lines.join("\n");
}

export function renderToolResult(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";

  try {
    const parsed = JSON.parse(trimmed) as NeedsUserInputPayload & {
      content?: Array<{ type?: string; text?: string }>;
    };
    if (parsed?.type === "needs_user_input") {
      const lines: string[] = [];
      if (parsed.context) {
        lines.push(`需要你补充信息：${parsed.context}`);
      }
      if (Array.isArray(parsed.questions) && parsed.questions.length > 0) {
        lines.push("");
        for (const item of parsed.questions) {
          lines.push(`${item.header}: ${item.question}`);
          for (const opt of item.options ?? []) {
            lines.push(`- ${opt.label}`);
          }
          lines.push("");
        }
      }
      if (parsed.allow_freeform) {
        lines.push("也可以直接描述你的具体需求。");
      }
      return lines.join("\n");
    }

    if (Array.isArray(parsed?.content)) {
      const texts = parsed.content
        .filter((b) => b && (b.type === "text" || typeof b.text === "string"))
        .map((b) => (b.text ?? "").trim())
        .filter(Boolean);
      if (texts.length > 0) {
        return texts.join("\n\n");
      }
    }

    return JSON.stringify(parsed, null, 2);
  } catch {
    return trimmed;
  }
}

export function parseNeedsUserInput(raw: string): NeedsUserInputPayload | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const parsed = JSON.parse(trimmed) as NeedsUserInputPayload;
    if (
      parsed?.type === "needs_user_input" &&
      Array.isArray(parsed.questions) &&
      parsed.questions.length > 0
    ) {
      return {
        type: parsed.type,
        context: parsed.context,
        allow_freeform: parsed.allow_freeform ?? true,
        questions: parsed.questions,
      };
    }
  } catch {
    return null;
  }
  return null;
}

export function parsePlanModeChange(raw: string): PlanModeChangePayload | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  try {
    const parsed = JSON.parse(trimmed) as PlanModeChangePayload;
    if (parsed?.type === "plan_mode_change" && parsed.mode) {
      return parsed;
    }
  } catch {
    return null;
  }
  return null;
}
