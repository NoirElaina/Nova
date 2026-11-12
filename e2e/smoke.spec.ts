import { expect, test } from "@playwright/test";

test.describe("Nova Desktop UI smoke (Mock Tauri IPC)", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      let nextCallbackId = 1;
      let nextListenerId = 1;
      let nextMsgId = 100;
      let nextConvSeq = 1;

      const callbacks = new Map<number, (payload: unknown) => void>();
      const listeners = new Map<string, Map<number, number>>();

      const state: {
        conversations: Array<{
          id: string;
          title: string;
          updatedAt: number;
          pinnedAt: number | null;
          workspacePath: string | null;
          activeAgentId: string | null;
        }>;
        messagesByConv: Record<
          string,
          Array<{
            id: number;
            role: string;
            content: string;
            reasoning?: string;
            tokenUsage?: number;
          }>
        >;
      } = {
        conversations: [],
        messagesByConv: {},
      };

      const defaultSettings = {
        provider: "openai",
        customModels: {},
        providerProfiles: {
          openai: {
            displayName: "Mock OpenAI",
            apiFormat: "openai",
            apiKey: "",
            baseUrl: "http://127.0.0.1:0/v1",
            model: "gpt-4.1",
            anthropicThinkingEnabled: false,
            anthropicThinkingBudgetTokens: null,
            stopSequences: [],
          },
        },
        providerOrder: ["openai"],
        modelContextWindows: { "gpt-4.1": 128000 },
        disabledSkills: [],
        uiLanguage: "zh-CN",
        uiTheme: "light",
        enableAppLog: false,
        approvalPolicy: "on_request",
        progressiveToolDisclosure: true,
        terminalShell: null,
      };

      function emitEvent(eventName: string, payload: unknown) {
        const eventMap = listeners.get(eventName);
        if (!eventMap) return;
        for (const [eventId, cbId] of eventMap.entries()) {
          const cb = callbacks.get(cbId);
          if (cb) {
            cb({ event: eventName, id: eventId, payload });
          }
        }
      }

      const w = window as unknown as Record<string, unknown>;
      w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener(eventName: string, eventId: number) {
          const eventMap = listeners.get(eventName);
          eventMap?.delete(eventId);
        },
      };

      w.__TAURI_INTERNALS__ = {
        transformCallback(callback: (payload: unknown) => void, once = false) {
          const id = nextCallbackId++;
          callbacks.set(id, (payload) => {
            if (once) callbacks.delete(id);
            callback(payload);
          });
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        convertFileSrc(filePath: string) {
          return filePath;
        },
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        async invoke(cmd: string, args: Record<string, unknown> = {}) {
          switch (cmd) {
            case "plugin:event|listen": {
              const eventName = String(args.event ?? "");
              const handlerId = Number(args.handler ?? 0);
              const eventId = nextListenerId++;
              if (!listeners.has(eventName)) {
                listeners.set(eventName, new Map());
              }
              listeners.get(eventName)!.set(eventId, handlerId);
              return eventId;
            }
            case "plugin:event|unlisten": {
              const eventName = String(args.event ?? "");
              const eventId = Number(args.eventId ?? 0);
              listeners.get(eventName)?.delete(eventId);
              return null;
            }
            case "get_settings":
              return structuredClone(defaultSettings);
            case "save_settings":
              return null;
            case "get_model_window_tokens":
              return 128000;
            case "estimate_text_tokens":
              return Math.ceil(String(args.text ?? "").length / 4);
            case "list_conversations":
              return state.conversations;
            case "create_conversation": {
              const id = `conv-smoke-${nextConvSeq++}`;
              const title = String(args.title || "Smoke Chat").slice(0, 40);
              const conv = {
                id,
                title,
                updatedAt: Date.now(),
                pinnedAt: null,
                workspacePath: (args.workspacePath as string | null) ?? null,
                activeAgentId: null,
              };
              state.conversations.unshift(conv);
              state.messagesByConv[id] = [];
              return conv;
            }
            case "load_history": {
              const convId = String(args.conversationId ?? "");
              return state.messagesByConv[convId] ?? [];
            }
            case "load_conversation_tool_logs":
            case "list_session_files":
            case "list_skills":
            case "list_todos":
            case "list_background_jobs":
            case "list_scheduled_tasks":
            case "get_available_terminals":
            case "get_agent_graph":
            case "get_board_channels":
            case "get_board_posts":
            case "list_memory_entries":
            case "list_permission_rules":
            case "get_mcp_server_statuses":
            case "get_model_catalog":
            case "list_token_usage":
            case "list_agent_bundles":
              return [];
            case "get_conversation_turn_traces":
              return [
                {
                  turnId: "turn-e2e-1",
                  traceId: "trace-e2e-1",
                  seq: 1,
                  startedAt: Date.now() - 100,
                  endedAt: Date.now(),
                  stopReason: "end_turn",
                  wireCalls: [
                    {
                      traceId: "trace-e2e-1",
                      request: { model: "gpt-4.1", messages: [{ role: "user", content: "Hello" }] },
                      response: { content: "Mock assistant response for E2E smoke." },
                    },
                  ],
                },
              ];
            case "get_conversation_usage":
              return {
                requests: 1,
                inputTokens: 12,
                outputTokens: 8,
                cacheReadTokens: 0,
                cacheCreationTokens: 0,
                totalTokens: 20,
                totalCostUsd: "0.0001",
              };
            case "get_usage_stats":
              return {
                total_tokens: 0,
                total_cost_usd: "0",
                requests: 0,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
              };
            case "get_model_catalog_stats":
              return { total: 0, updatedAt: null };
            case "get_git_repo_status":
            case "get_workspace_git_status":
              return { isGitRepo: false, rootPath: null, branch: null };
            case "get_conversation_agent":
            case "get_conversation_plan":
            case "load_browser_tab_state":
              return null;
            case "get_hooks_toml":
              return "";
            case "append_plain_chat_message": {
              const convId = String(args.conversationId ?? "");
              const list = (state.messagesByConv[convId] ??= []);
              list.push({
                id: nextMsgId++,
                role: String(args.role ?? "user"),
                content: String(args.content ?? ""),
              });
              return null;
            }
            case "send_modern_agent_turn": {
              const convId = String(args.conversationId ?? "");
              const prompt = String(args.prompt ?? "");
              const list = (state.messagesByConv[convId] ??= []);
              list.push({
                id: nextMsgId++,
                role: "user",
                content: prompt,
              });
              list.push({
                id: nextMsgId++,
                role: "assistant",
                content: "Mock assistant response for E2E smoke.",
                tokenUsage: 20,
              });

              const turnId = "turn-e2e-1";
              const traceId = "trace-e2e-1";
              setTimeout(() => {
                emitEvent("agent-event", {
                  kind: "turn_started",
                  payload: { turn_id: turnId, conversation_id: convId, timestamp: Date.now(), trace_id: traceId },
                });
              }, 10);
              setTimeout(() => {
                emitEvent("agent-event", {
                  kind: "text_delta",
                  payload: { turn_id: turnId, delta: "Mock assistant response for E2E smoke.", trace_id: traceId },
                });
              }, 20);
              setTimeout(() => {
                emitEvent("agent-event", {
                  kind: "turn_finished",
                  payload: { turn_id: turnId, stop_reason: "end_turn", total_tokens: 20, trace_id: traceId },
                });
              }, 35);
              return null;
            }
            default:
              return null;
          }
        },
      };
    });
  });

  test("renders welcome screen, sends a mocked turn, and navigates primary views without page errors", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (err) => {
      pageErrors.push(err.message);
    });

    await page.goto("/");

    await expect(page.getByRole("heading", { name: "Back at it, Nova" })).toBeVisible();

    const composer = page.getByPlaceholder("Message Nova...");
    await expect(composer).toBeVisible();
    await composer.fill("Hello Nova smoke test");
    await composer.press("Enter");

    await expect(page.getByRole("button", { name: "Hello Nova smoke test" })).toBeVisible();
    await expect(page.getByRole("main").getByText("Hello Nova smoke test")).toBeVisible();
    await expect(page.getByRole("main").getByText("Mock assistant response for E2E smoke.")).toBeVisible();

    // 校验其他核心路由页签可正常挂载渲染且无未捕获 JS 异常
    await page.goto("/#/agent");
    await expect(page.locator("#app")).toBeVisible();

    await page.goto("/#/settings");
    await expect(page.locator("#app")).toBeVisible();

    await page.goto("/#/chat");
    await expect(page.locator("#app")).toBeVisible();

    expect(pageErrors).toEqual([]);
  });
});
