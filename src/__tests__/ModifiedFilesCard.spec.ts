import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia, type Pinia } from "pinia";
import { mount } from "@vue/test-utils";
import AssistantTranscript from "@/components/chat/messages/AssistantTranscript.vue";
import ModifiedFilesCard from "@/components/chat/messages/ModifiedFilesCard.vue";
import {
  collectTranscriptToolEntries,
  filterToolEntriesForSegments,
} from "@/features/chat/utils/assistant-transcript";
import type { AssistantTranscriptSegment, ToolExecutionEntry } from "@/lib/chat-types";

function makeWriteEntry(id: string, filePath: string, content: string): ToolExecutionEntry {
  return {
    id,
    toolName: "write_file",
    input: JSON.stringify({ file_path: filePath, content }),
    result: "Written successfully",
    status: "completed",
    startedAt: 1000,
    finishedAt: 1020,
  };
}

function makeEditEntry(
  id: string,
  filePath: string,
  oldString: string,
  newString: string,
): ToolExecutionEntry {
  return {
    id,
    toolName: "edit_file",
    input: JSON.stringify({
      file_path: filePath,
      old_string: oldString,
      new_string: newString,
    }),
    result: "Edited successfully",
    status: "completed",
    startedAt: 2000,
    finishedAt: 2050,
  };
}

describe("ModifiedFilesCard multi-turn isolation", () => {
  let pinia: Pinia;

  beforeEach(() => {
    pinia = createPinia();
    setActivePinia(pinia);
  });

  it("renders file groups and hunks from entries matching file-change tools", () => {
    const writeEntry = makeWriteEntry("t-write", "src/alpha.ts", "export const a = 1;\n");
    const editEntry = makeEditEntry("t-edit", "src/beta.ts", "const x = 1;", "const x = 2;");
    const readOnlyEntry: ToolExecutionEntry = {
      id: "t-read",
      toolName: "read_file",
      input: JSON.stringify({ path: "README.md" }),
      result: "ok",
      status: "completed",
      startedAt: 1500,
      finishedAt: 1510,
    };

    const wrapper = mount(ModifiedFilesCard, {
      props: {
        entries: [writeEntry, editEntry, readOnlyEntry],
      },
    });

    expect(wrapper.find(".modified-files-card").exists()).toBe(true);
    expect(wrapper.text()).toContain("src/alpha.ts");
    expect(wrapper.text()).toContain("src/beta.ts");
    expect(wrapper.text()).toContain("write_file");
    expect(wrapper.text()).toContain("edit_file");
    expect(wrapper.text()).not.toContain("README.md");
  });

  it("isolates Turn 1 modified files from Turn 2 assistant bubble when Turn 2 has no tool segments", () => {
    const turn1Entry = makeWriteEntry("turn1-write", "src/turn1-only.ts", "const turn = 1;");
    const turn2ReadOnly: ToolExecutionEntry = {
      id: "turn2-read",
      toolName: "read_file",
      input: JSON.stringify({ path: "src/turn1-only.ts" }),
      result: "const turn = 1;",
      status: "completed",
      startedAt: 3000,
      finishedAt: 3010,
    };
    const conversationLogs = [turn1Entry, turn2ReadOnly];

    const turn1Segments: AssistantTranscriptSegment[] = [
      { type: "tools", toolIds: ["turn1-write"] },
      { type: "text", text: "Created turn1-only.ts" },
    ];
    const turn2Segments: AssistantTranscriptSegment[] = [
      { type: "text", text: "Turn 2 answer without file edits" },
    ];

    expect(filterToolEntriesForSegments(conversationLogs, turn1Segments).map((e) => e.id)).toEqual([
      "turn1-write",
    ]);
    expect(filterToolEntriesForSegments(conversationLogs, turn2Segments)).toEqual([]);
    expect(collectTranscriptToolEntries(turn2Segments, conversationLogs)).toEqual([]);

    const turn1Transcript = mount(AssistantTranscript, {
      global: {
        plugins: [pinia],
      },
      props: {
        segments: turn1Segments,
        entries: conversationLogs,
      },
    });
    const turn2Transcript = mount(AssistantTranscript, {
      global: {
        plugins: [pinia],
      },
      props: {
        segments: turn2Segments,
        entries: conversationLogs,
      },
    });

    expect(turn1Transcript.find(".modified-files-card").exists()).toBe(true);
    expect(turn1Transcript.text()).toContain("src/turn1-only.ts");

    // 关键回归保障：即使传入全会话 logs，无工具段的 Turn 2 绝不能渲染 Turn 1 的修改文件卡片
    expect(turn2Transcript.find(".modified-files-card").exists()).toBe(false);
    expect(turn2Transcript.text()).not.toContain("src/turn1-only.ts");
  });
});
