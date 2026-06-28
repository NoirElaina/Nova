function readStringField(input: Record<string, unknown> | null, keys: string[]): string | null {
  if (!input) return null;
  for (const key of keys) {
    const value = input[key];
    if (typeof value === "string" && value.trim()) {
      return value.trim();
    }
  }
  return null;
}

function truncateText(text: string, maxLen: number): string {
  const v = (text || "").trim();
  if (!v) return v;
  return v.length > maxLen ? `${v.slice(0, maxLen)}...` : v;
}

export function summarizeToolInfo(toolName: string, rawInput: string): string | null {
  let parsed: Record<string, unknown> | null = null;
  if (rawInput.trim()) {
    try {
      parsed = JSON.parse(rawInput) as Record<string, unknown>;
    } catch {
      parsed = null;
    }
  }

  const lower = (toolName || "").toLowerCase();

  if (lower === "bash" || lower.includes("shell")) {
    const command = readStringField(parsed, ["command", "cmd", "script"]);
    if (!command) return "shell action";
    const first = command.split(/\s+/).filter(Boolean)[0] || "unknown";
    return `command=${first}`;
  }

  if (
    lower.includes("browser") ||
    lower.includes("web_fetch") ||
    lower.includes("web_search") ||
    lower.includes("navigate")
  ) {
    const url = readStringField(parsed, ["url", "website", "uri"]);
    if (url) {
      try {
        const u = new URL(url);
        return `site=${truncateText(`${u.host}${u.pathname}`, 72)}`;
      } catch {
        return `site=${truncateText(url, 72)}`;
      }
    }
    const query = readStringField(parsed, ["query", "text"]);
    if (query) {
      return `query=${truncateText(query, 48)}`;
    }
    return "browser action";
  }

  if (lower === "multiedit") {
    const path = readStringField(parsed, ["file_path", "path", "filePath", "uri"]);
    const edits = parsed?.["edits"];
    const count = Array.isArray(edits) ? edits.length : 0;
    return path
      ? count > 0
        ? `path=${truncateText(path, 64)} · ${count} edit${count > 1 ? "s" : ""}`
        : `path=${truncateText(path, 64)}`
      : "file operation";
  }

  if (lower === "edit" || lower.includes("replace_string")) {
    const path = readStringField(parsed, ["file_path", "path", "filePath", "uri"]);
    const oldStr = readStringField(parsed, ["old_string", "oldString", "search", "find"]);
    const newStr = readStringField(parsed, ["new_string", "newString", "replace", "replacement"]);
    if (!path) return "file operation";
    if (oldStr && newStr) {
      return `path=${truncateText(path, 64)} · "${truncateText(oldStr, 32)}" → "${truncateText(newStr, 32)}"`;
    }
    return `path=${truncateText(path, 64)}`;
  }

  if (
    lower === "read" ||
    lower === "write" ||
    lower.includes("file_read") ||
    lower.includes("write_file") ||
    lower.includes("file_edit")
  ) {
    const path = readStringField(parsed, ["file_path", "path", "filePath", "uri"]);
    return path ? `path=${truncateText(path, 64)}` : "file operation";
  }

  if (lower === "glob") {
    const pattern = readStringField(parsed, ["pattern"]);
    return pattern ? `pattern=${truncateText(pattern, 48)}` : "glob";
  }

  if (lower === "grep") {
    const pattern = readStringField(parsed, ["pattern"]);
    return pattern ? `pattern=${truncateText(pattern, 48)}` : "grep";
  }

  if (lower.startsWith("mcp__")) {
    const url = readStringField(parsed, ["url", "uri"]);
    if (url) return `mcp site=${truncateText(url, 64)}`;
    if (parsed) {
      const keys = Object.keys(parsed).slice(0, 3).join(",");
      return keys ? `mcp args=${keys}` : "mcp call";
    }
    return "mcp call";
  }

  if (parsed) {
    const keys = Object.keys(parsed).slice(0, 2).join(",");
    if (keys) return `args=${keys}`;
  }

  return null;
}
