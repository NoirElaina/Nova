import type {
  ToolExecutionEntry,
  ToolTurnCategoryCount,
  ToolTurnSummary,
} from "../../../lib/chat-types";

type ToolCategory = {
  key: string;
  label: string;
};

const categoryCatalog: ToolCategory[] = [
  { key: "read", label: "读取文件" },
  { key: "write", label: "写入文件" },
  { key: "shell", label: "执行命令" },
  { key: "search", label: "搜索代码" },
  { key: "web", label: "访问网页" },
  { key: "goal", label: "目标管理" },
  { key: "mcp", label: "MCP 工具" },
  { key: "other", label: "其他工具" },
];

function categorizeTool(toolName: string): ToolCategory {
  const lower = (toolName || "").toLowerCase();

  if (
    lower.includes("file_read") ||
    lower.includes("read_file") ||
    lower.includes("readmcpresource")
  ) {
    return categoryCatalog[0];
  }

  if (
    lower.includes("write_file") ||
    lower.includes("file_edit") ||
    lower.includes("replace") ||
    lower.includes("remember") ||
    lower.includes("config")
  ) {
    return categoryCatalog[1];
  }

  if (
    lower.includes("bash") ||
    lower.includes("powershell") ||
    lower.includes("shell")
  ) {
    return categoryCatalog[2];
  }

  if (
    lower.includes("grep") ||
    lower.includes("glob") ||
    lower.includes("search")
  ) {
    return categoryCatalog[3];
  }

  if (
    lower.includes("web_fetch") ||
    lower.includes("web_search") ||
    lower.includes("browser") ||
    lower.includes("computeruse")
  ) {
    return categoryCatalog[4];
  }

  if (
    lower.includes("goal") ||
    lower.includes("plan")
  ) {
    return categoryCatalog[5];
  }

  if (lower.startsWith("mcp__") || lower.includes("mcp")) {
    return categoryCatalog[6];
  }

  return categoryCatalog[7];
}

export function buildToolTurnSummary(entries: ToolExecutionEntry[]): ToolTurnSummary | undefined {
  if (!entries.length) {
    return undefined;
  }

  const counts = new Map<string, ToolTurnCategoryCount>();
  for (const entry of entries) {
    const category = categorizeTool(entry.toolName);
    const existing = counts.get(category.key);
    if (existing) {
      existing.count += 1;
    } else {
      counts.set(category.key, {
        label: category.label,
        count: 1,
      });
    }
  }

  const categoryCounts = [...counts.values()].sort((a, b) => b.count - a.count);

  return {
    totalCalls: entries.length,
    categoryCounts,
    entries: entries.map((entry) => ({ ...entry })),
  };
}

export function renderToolTurnSummaryLine(summary: ToolTurnSummary): string {
  return `本轮调用了 ${summary.totalCalls} 个工具`;
}

export function renderToolTurnCategoryLine(summary: ToolTurnSummary): string {
  return summary.categoryCounts
    .slice(0, 4)
    .map((item) => `${item.label} ${item.count} 次`)
    .join("，");
}
