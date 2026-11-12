import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import type { BackgroundOutput } from "@/lib/protocol";

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

vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class MockTerminal {
    rows = 24;
    cols = 80;
    options = {};
    loadAddon() {}
    open() {}
    attachCustomKeyEventHandler() {}
    onData() {
      return { dispose() {} };
    }
    scrollToBottom() {}
    focus() {}
    write() {}
    hasSelection() {
      return false;
    }
    getSelection() {
      return "";
    }
    clearSelection() {}
    dispose() {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class MockFitAddon {
    fit() {}
  },
}));

import TerminalTab from "@/components/chat/workspace/TerminalTab.vue";

function createDeferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeJob(overrides: Partial<BackgroundOutput> = {}): BackgroundOutput {
  return {
    id: "job-1",
    pid: 4242,
    command: "npm run dev",
    cwd: "D:/repo",
    running: true,
    finished: false,
    exitCode: null,
    killed: false,
    timedOut: false,
    stdout: "server booting",
    stderr: "",
    truncated: false,
    elapsedMs: 250,
    ttlMs: 300000,
    remainingMs: 300000,
    ...overrides,
  };
}

describe("TerminalTab polling & kill race protection", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockImplementation(async () => () => {});

    globalThis.ResizeObserver = class MockResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("prevents an in-flight read_background_job_output poll from overwriting killJob terminal state", async () => {
    const runningJob = makeJob({ id: "job-1", running: true, stdout: "initial line" });
    const killedJob = makeJob({
      id: "job-1",
      running: false,
      finished: true,
      killed: true,
      stdout: "final killed output",
      elapsedMs: 900,
    });
    const stalePollOutput = makeJob({
      id: "job-1",
      running: true,
      finished: false,
      killed: false,
      stdout: "STALE RUNNING OUTPUT",
    });

    const staleOutputDeferred = createDeferred<BackgroundOutput>();
    let currentJobs = [runningJob];
    let readCallCount = 0;

    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "user_terminal_start":
          return {
            sessionId: "term-sess-1",
            cwd: "D:/repo",
            shellName: "pwsh",
          };
        case "user_terminal_resize":
        case "user_terminal_stop":
          return null;
        case "list_background_jobs":
          return currentJobs;
        case "read_background_job_output":
          readCallCount += 1;
          return staleOutputDeferred.promise;
        case "kill_background_job":
          currentJobs = [killedJob];
          return killedJob;
        default:
          return null;
      }
    });

    const wrapper = mount(TerminalTab, {
      props: {
        conversationId: "conv-race",
        visible: true,
      },
    });

    await flushPromises();
    expect(wrapper.text()).toContain("npm run dev");

    // 选中运行中后台任务，发起挂起的 read_background_job_output 轮询
    const jobRow = wrapper.find('[title*="#job-1"]');
    expect(jobRow.exists()).toBe(true);
    await jobRow.trigger("click");
    await flushPromises();
    expect(readCallCount).toBe(1);

    // 在读取响应返回之前点击“终止”
    const stopButton = wrapper
      .findAll("button")
      .find((btn) => btn.text().includes("终止"));
    expect(stopButton).toBeDefined();
    await stopButton!.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("final killed output");
    expect(wrapper.text()).toContain("已终止");

    // 迟到的旧轮询结果回到前端，必须被 outputRequestSeq 丢弃
    staleOutputDeferred.resolve(stalePollOutput);
    await flushPromises();

    expect(wrapper.text()).toContain("final killed output");
    expect(wrapper.text()).not.toContain("STALE RUNNING OUTPUT");
    expect(wrapper.text()).toContain("已终止");

    wrapper.unmount();
  });

  it("drops stale list_background_jobs responses after switching conversations", async () => {
    const conv1JobsDeferred = createDeferred<BackgroundOutput[]>();

    invokeMock.mockImplementation(async (cmd: string, args?: { conversationId?: string }) => {
      switch (cmd) {
        case "user_terminal_start":
          return {
            sessionId: `term-${args?.conversationId ?? "default"}`,
            cwd: "D:/repo",
            shellName: "pwsh",
          };
        case "user_terminal_resize":
        case "user_terminal_stop":
          return null;
        case "list_background_jobs":
          if (args?.conversationId === "conv-1") {
            return conv1JobsDeferred.promise;
          }
          return [];
        default:
          return null;
      }
    });

    const wrapper = mount(TerminalTab, {
      props: {
        conversationId: "conv-1",
        visible: true,
      },
    });
    await flushPromises();

    // conv-1 的轮询仍在途时切换至 conv-2
    await wrapper.setProps({ conversationId: "conv-2" });
    await flushPromises();

    // 旧会话的 list_background_jobs 迟到返回，不能污染 conv-2 列表
    conv1JobsDeferred.resolve([
      makeJob({ id: "job-stale", command: "stale-conv1-cmd" }),
    ]);
    await flushPromises();

    expect(wrapper.text()).not.toContain("stale-conv1-cmd");
    expect(wrapper.text()).toContain("暂无后台任务");

    wrapper.unmount();
  });
});
