import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

import { useConversationStore } from "@/stores/conversation";
import { useAgentSessionStore } from "@/stores/agentSession";

describe("useAgentSessionStore", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(async () => () => {});
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_session_files":
        case "load_history":
        case "load_conversation_tool_logs":
          return [];
        case "get_conversation_usage":
          return null;
        default:
          return null;
      }
    });
  });

  it("isolates streaming deltas, tool logs, and errors across concurrent conversations by turnId", () => {
    const conversationStore = useConversationStore();
    const sessionStore = useAgentSessionStore();

    conversationStore.selectConversation("conv-a");

    sessionStore.handleTurnStarted("turn-a", "conv-a");
    sessionStore.handleTurnStarted("turn-b", "conv-b");

    sessionStore.handleThinkingDelta("think-a ", "turn-a");
    sessionStore.handleTextDelta("Reply A", "turn-a");
    sessionStore.handleTextDelta("Reply B", "turn-b");
    sessionStore.handleToolRequested("call-b1", "read_file", { path: "b.ts" }, "turn-b");
    sessionStore.handleToolCompleted("call-b1", "read_file", "ok", false, 12, "turn-b");

    const sessionA = sessionStore.getSession("conv-a");
    const sessionB = sessionStore.getSession("conv-b");

    expect(sessionStore.activeSession.conversationId).toBe("conv-a");
    expect(sessionA.assistantReasoning).toBe("think-a ");
    expect(sessionA.assistantResponse).toBe("Reply A");
    expect(sessionA.toolExecutionLogs).toHaveLength(0);
    expect(sessionA.isGenerating).toBe(true);

    expect(sessionB.assistantResponse).toBe("Reply B");
    expect(sessionB.toolExecutionLogs).toHaveLength(1);
    expect(sessionB.toolExecutionLogs[0]).toMatchObject({
      id: "call-b1",
      toolName: "read_file",
      status: "completed",
      result: "ok",
    });

    sessionStore.handleTurnError("rate limit exceeded", "turn-b", "usageLimitExceeded");
    expect(sessionB.isGenerating).toBe(false);
    expect(sessionB.chatError).toBe("rate limit exceeded");
    expect(sessionB.chatErrorInfo).toBe("usageLimitExceeded");

    // 另一会话不应受 conv-b 报错影响
    expect(sessionA.isGenerating).toBe(true);
    expect(sessionA.chatError).toBeNull();
  });

  it("clears turn runtime on error dismissal and clearActiveTurnRuntime without erasing persisted messages", async () => {
    const conversationStore = useConversationStore();
    const sessionStore = useAgentSessionStore();

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "load_history") {
        return [
          { id: 1, role: "user", content: "Hi" },
          { id: 2, role: "assistant", content: "Hello!" },
        ];
      }
      if (cmd === "load_conversation_tool_logs") {
        return [
          {
            id: "tool-1",
            toolName: "write_file",
            input: JSON.stringify({ file_path: "a.ts", content: "x" }),
            result: "ok",
            status: "completed",
            startedAt: 100,
            finishedAt: 110,
          },
        ];
      }
      return [];
    });

    conversationStore.selectConversation("conv-1");
    await sessionStore.loadConversationMessages("conv-1");
    expect(sessionStore.activeSession.messages).toHaveLength(2);
    expect(sessionStore.activeSession.toolExecutionLogs).toHaveLength(1);

    sessionStore.handleTurnStarted("turn-1", "conv-1");
    sessionStore.handleTextDelta("partial", "turn-1");
    sessionStore.handleTurnError("unauthorized", "turn-1", "unauthorized");

    expect(sessionStore.activeSession.chatError).toBe("unauthorized");
    sessionStore.dismissChatError("conv-1");
    expect(sessionStore.activeSession.chatError).toBeNull();
    expect(sessionStore.activeSession.chatErrorInfo).toBeNull();

    sessionStore.clearActiveTurnRuntime("conv-1");
    expect(sessionStore.activeSession.isGenerating).toBe(false);
    expect(sessionStore.activeSession.assistantResponse).toBe("");
    expect(sessionStore.activeSession.assistantSegments).toEqual([]);
    expect(sessionStore.activeSession.messages).toHaveLength(2);
  });

  it("finalizes unclosed running tools according to stopReason on handleTurnFinished", async () => {
    const conversationStore = useConversationStore();
    const sessionStore = useAgentSessionStore();
    conversationStore.selectConversation("conv-fin");

    sessionStore.handleTurnStarted("turn-fin", "conv-fin");
    sessionStore.handleToolRequested("call-1", "exec_command", { cmd: "sleep 1" }, "turn-fin");
    expect(sessionStore.getSession("conv-fin").toolExecutionLogs[0].status).toBe("running");

    await sessionStore.handleTurnFinished("turn-fin", "end_turn", "conv-fin");

    const session = sessionStore.getSession("conv-fin");
    expect(session.isGenerating).toBe(false);
    expect(session.toolExecutionLogs[0].status).toBe("error");

    sessionStore.handleTurnStarted("turn-cancel", "conv-fin");
    sessionStore.handleToolRequested("call-2", "exec_command", { cmd: "sleep 2" }, "turn-cancel");
    await sessionStore.handleTurnFinished("turn-cancel", "cancelled", "conv-fin");

    expect(session.toolExecutionLogs[1].status).toBe("cancelled");
  });
});
