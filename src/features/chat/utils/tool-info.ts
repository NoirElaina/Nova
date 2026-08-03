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

function parseToolInputJson(rawInput: string): Record<string, unknown> | null {
  if (!rawInput.trim()) return null;
  try {
    return JSON.parse(rawInput) as Record<string, unknown>;
  } catch {
    return null;
  }
}

/**
 * 从流式未完成的 tool JSON 里尽量抠出字符串字段。
 * 模型通常先输出 file_path，再灌 content；完整 JSON.parse 在 content 中途会失败。
 */
function extractJsonStringField(raw: string, keys: string[]): string | null {
  if (!raw) return null;
  for (const key of keys) {
    // "file_path": "...."  允许转义字符；遇到未闭合引号时取到字符串末尾
    const re = new RegExp(
      `"(?:${key})"\\s*:\\s*"((?:\\\\.|[^"\\\\])*)"?`,
      "i",
    );
    const match = raw.match(re);
    if (!match) continue;
    const rawValue = match[1] ?? "";
    try {
      // 用 JSON 反转义，保证 \\ 与 \" 正确
      return JSON.parse(`"${rawValue}"`) as string;
    } catch {
      // 未闭合或半截转义：退回字面量（去掉常见转义）
      return rawValue
        .replace(/\\n/g, "\n")
        .replace(/\\r/g, "\r")
        .replace(/\\t/g, "\t")
        .replace(/\\"/g, '"')
        .replace(/\\\\/g, "\\");
    }
  }
  return null;
}

const PATH_KEYS = ["file_path", "path", "filePath", "uri", "target_path", "target"];

/** 完整或流式 input 中解析路径 */
export function extractToolPath(rawInput: string): string | null {
  const parsed = parseToolInputJson(rawInput);
  const fromJson = readStringField(parsed, PATH_KEYS);
  if (fromJson) return fromJson;
  const partial = extractJsonStringField(rawInput, PATH_KEYS);
  return partial?.trim() || null;
}

/** 仅文件名，用于紧凑展示 */
export function fileNameFromPath(filePath: string): string {
  const normalized = filePath.replace(/\\/g, "/");
  const parts = normalized.split("/").filter(Boolean);
  return parts[parts.length - 1] || filePath;
}

/** 流式 content 大致字节进度（未完成 JSON 也可估） */
export function estimateStreamingContentChars(rawInput: string): number | null {
  const parsed = parseToolInputJson(rawInput);
  const full = readStringField(parsed, ["content", "new_content", "text", "new_string", "newString"]);
  if (full != null) return full.length;

  // 半截："content": "....
  const match = rawInput.match(
    /"(?:content|new_content|text|new_string|newString)"\s*:\s*"((?:\\.|[^"\\])*)/i,
  );
  if (!match) return null;
  return match[1]?.length ?? 0;
}

function formatPathSummary(path: string, maxLen = 72): string {
  const name = fileNameFromPath(path);
  if (path.length <= maxLen) {
    return path.includes("/") || path.includes("\\") ? `${name} · ${path}` : name;
  }
  // 长路径：文件名 + 截断路径
  const room = Math.max(24, maxLen - name.length - 3);
  const clipped =
    path.length > room ? `…${path.slice(-(room - 1))}` : path;
  return `${name} · ${clipped}`;
}

export function summarizeToolInfo(toolName: string, rawInput: string): string | null {
  const parsed = parseToolInputJson(rawInput);
  const lower = (toolName || "").toLowerCase();

  if (lower === "bash" || lower.includes("shell")) {
    const command =
      readStringField(parsed, ["command", "cmd", "script"]) ||
      extractJsonStringField(rawInput, ["command", "cmd", "script"]);
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
    const url =
      readStringField(parsed, ["url", "website", "uri"]) ||
      extractJsonStringField(rawInput, ["url", "website", "uri"]);
    if (url) {
      try {
        const u = new URL(url);
        return `site=${truncateText(`${u.host}${u.pathname}`, 72)}`;
      } catch {
        return `site=${truncateText(url, 72)}`;
      }
    }
    const query =
      readStringField(parsed, ["query", "text"]) ||
      extractJsonStringField(rawInput, ["query", "text"]);
    if (query) {
      return `query=${truncateText(query, 48)}`;
    }
    return "browser action";
  }

  if (lower === "multiedit") {
    const path = extractToolPath(rawInput);
    const edits = parsed?.["edits"];
    const count = Array.isArray(edits) ? edits.length : 0;
    if (!path) return count > 0 ? `${count} edit${count > 1 ? "s" : ""}` : null;
    const base = formatPathSummary(path, 64);
    return count > 0 ? `${base} · ${count} edit${count > 1 ? "s" : ""}` : base;
  }

  if (lower === "edit" || lower.includes("replace_string") || lower.includes("str_replace")) {
    const path = extractToolPath(rawInput);
    if (!path) return null;
    return formatPathSummary(path, 72);
  }

  if (
    lower === "read" ||
    lower === "write" ||
    lower.includes("file_read") ||
    lower.includes("write_file") ||
    lower.includes("file_edit") ||
    lower.includes("create_file")
  ) {
    const path = extractToolPath(rawInput);
    if (!path) {
      // 写入流式中：尚无 path 时给明确状态，避免 “file operation” 盲盒
      if (lower.includes("write") || lower.includes("create")) {
        const chars = estimateStreamingContentChars(rawInput);
        if (chars != null && chars > 0) {
          return `写入中 · ${chars.toLocaleString()} chars`;
        }
        return "准备写入…";
      }
      if (lower.includes("read")) return "准备读取…";
      return null;
    }
    const base = formatPathSummary(path, 72);
    if ((lower.includes("write") || lower.includes("create")) && !parsed) {
      const chars = estimateStreamingContentChars(rawInput);
      if (chars != null && chars > 0) {
        return `${base} · ${chars.toLocaleString()} chars`;
      }
    }
    return base;
  }

  if (lower === "glob") {
    const pattern =
      readStringField(parsed, ["pattern"]) ||
      extractJsonStringField(rawInput, ["pattern"]);
    return pattern ? `pattern=${truncateText(pattern, 48)}` : "glob";
  }

  if (lower === "grep") {
    const pattern =
      readStringField(parsed, ["pattern"]) ||
      extractJsonStringField(rawInput, ["pattern"]);
    return pattern ? `pattern=${truncateText(pattern, 48)}` : "grep";
  }

  if (lower.startsWith("mcp__")) {
    const url =
      readStringField(parsed, ["url", "uri"]) ||
      extractJsonStringField(rawInput, ["url", "uri"]);
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
