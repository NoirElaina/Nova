import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia, type Pinia } from "pinia";

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

import {
  resetChatControllerForTesting,
  useChatController,
} from "@/features/chat/controllers/useChatController";
import { useConversationStore } from "@/stores/conversation";
import { useAgentSessionStore } from "@/stores/agentSession";

function createDeferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("useChatController", () => {
  let pinia: Pinia;
  let hostWrapper: VueWrapper | null = null;

  function mountController() {
    let controller!: ReturnType<typeof useChatController>;
    const Host = defineComponent({
      setup() {
        controller = useChatController();
        return () => null;
      },
    });
    hostWrapper = mount(Host, {
      global: {
        plugins: [pinia],
      },
    });
    return controller;
  }

  beforeEach(() => {
    resetChatControllerForTesting();
    pinia = createPinia();
    setActivePinia(pinia);
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(async () => () => {});
  });

  afterEach(() => {
    hostWrapper?.unmount();
    hostWrapper = null;
    resetChatControllerForTesting();
  });

  it("cancels only the active conversation turn without global fallback", async () => {
    const cancelCalls: Array<{ conversationId: string | null }> = [];
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "list_conversations":
        case "list_session_files":
        case "load_history":
        case "load_conversation_tool_logs":
          return [];
        case "get_conversation_usage":
          return null;
        case "cancel_modern_agent_turn":
          cancelCalls.push({
            conversationId: (args?.conversationId as string | null) ?? null,
          });
          return args?.conversationId === null;
        default:
          return null;
      }
    });

    const controller = mountController();
    const conversationStore = useConversationStore();
    const sessionStore = useAgentSessionStore();

    conversationStore.selectConversation("conv-cancel");
    sessionStore.handleTurnStarted("turn-cancel", "conv-cancel");
    sessionStore.handleToolRequested("call-run-1", "exec_command", { command: "sleep 30" }, "turn-cancel");

    expect(controller.isGenerating.value).toBe(true);
    expect(controller.toolExecutionLogs.value[0].status).toBe("running");

    await controller.handleCancelGeneration();

    expect(controller.isGenerating.value).toBe(false);
    expect(controller.toolExecutionLogs.value[0].status).toBe("cancelled");
    expect(cancelCalls).toEqual([{ conversationId: "conv-cancel" }]);
  });

  it("guards against stale out-of-order conversation loads and clears draft state on handleNewChat", async () => {
    const oldHistoryDeferred = createDeferred<
      Array<{ id: number; role: string; content: string }>
    >();

    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      switch (cmd) {
        case "list_conversations":
          return [
            {
              id: "conv-old",
              title: "Old",
              updatedAt: 100,
              pinnedAt: null,
              workspacePath: "/old/workspace",
              activeAgentId: null,
            },
            {
              id: "conv-new",
              title: "New",
              updatedAt: 200,
              pinnedAt: null,
              workspacePath: "/new/workspace",
              activeAgentId: null,
            },
          ];
        case "load_history":
          if (args?.conversationId === "conv-old") {
            return oldHistoryDeferred.promise;
          }
          if (args?.conversationId === "conv-new") {
            return [
              { id: 10, role: "user", content: "New question" },
              { id: 11, role: "assistant", content: "New answer" },
            ];
          }
          return [];
        case "load_conversation_tool_logs":
        case "list_session_files":
          return [];
        case "get_conversation_usage":
          return null;
        default:
          return null;
      }
    });

    const controller = mountController();
    const conversationStore = useConversationStore();
    await conversationStore.loadConversations();

    // 连续切换：conv-old 请求未返回前，立刻切到 conv-new
    const oldLoadPromise = controller.handleSelectConversation("conv-old");
    await controller.handleSelectConversation("conv-new");

    expect(controller.activeConversationId.value).toBe("conv-new");
    expect(controller.activeWorkspacePath.value).toBe("/new/workspace");
    expect(controller.messages.value.map((m) => m.content)).toEqual([
      "New question",
      "New answer",
    ]);

    // 旧会话延迟返回：不得覆盖当前激活会话的 workspacePath 与展示消息
    oldHistoryDeferred.resolve([{ id: 1, role: "user", content: "Stale old question" }]);
    await oldLoadPromise;

    expect(controller.activeConversationId.value).toBe("conv-new");
    expect(controller.activeWorkspacePath.value).toBe("/new/workspace");
    expect(controller.messages.value.map((m) => m.content)).toEqual([
      "New question",
      "New answer",
    ]);

    // handleNewChat 回到干净草稿态
    await controller.handleNewChat();
    expect(controller.activeConversationId.value).toBe("");
    expect(controller.messages.value).toEqual([]);
    expect(controller.isGenerating.value).toBe(false);
  });

  it("updates contextTokens from ManualCompactOutcome snake_case fields", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_conversations":
        case "list_session_files":
        case "load_history":
        case "load_conversation_tool_logs":
          return [];
        case "get_conversation_usage":
          return null;
        case "manual_compact_conversation":
          return {
            before_tokens: 12000,
            after_tokens: 3400,
            saved_tokens: 8600,
            summary: "compacted summary",
          };
        default:
          return null;
      }
    });

    const controller = mountController();
    const conversationStore = useConversationStore();
    const sessionStore = useAgentSessionStore();
    conversationStore.selectConversation("conv-compact");

    await controller.handleCompactConversation();
    expect(sessionStore.getSession("conv-compact").contextTokens).toBe(3400);
  });
});
