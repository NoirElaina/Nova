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
    lower.includes("read") ||
    lower.includes("readmcpresource")
  ) {
    return categoryCatalog[0];
  }

  if (
    lower.includes("write") ||
    lower.includes("edit") ||
    lower.includes("remember") ||
    lower.includes("config")
  ) {
    return categoryCatalog[1];
  }

  if (
    lower === "bash" ||
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
  const readCount =
    summary.categoryCounts.find((item) => item.label === "读取文件")?.count ?? 0;
  if (readCount > 0) {
    return `Read ${readCount} ${readCount === 1 ? "file" : "files"}, used ${summary.totalCalls} ${summary.totalCalls === 1 ? "tool" : "tools"}`;
  }

  return `Used ${summary.totalCalls} ${summary.totalCalls === 1 ? "tool" : "tools"}`;
}
