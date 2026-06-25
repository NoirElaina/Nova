import type { ToolExecutionEntry } from "../../../lib/chat-types";

export type FileChangeKind = "edit" | "write";

export interface DiffLine {
  type: "context" | "add" | "del";
  text: string;
  oldLine?: number;
  newLine?: number;
}

export interface DiffHunkHeader {
  oldStart: number;
  oldLen: number;
  newStart: number;
  newLen: number;
}

export interface FileChangeHunk {
  toolName: string;
  kind: FileChangeKind;
  filePath: string;
  oldString?: string;
  newString?: string;
  content?: string;
  status: ToolExecutionEntry["status"];
  diff: DiffLine[];
  hunkHeader: DiffHunkHeader | null;
  oldTotalLines: number;
  newTotalLines: number;
  addedCount: number;
  removedCount: number;
  startedAt: number;
  finishedAt?: number;
}

export interface FileChangeGroup {
  filePath: string;
  hunks: FileChangeHunk[];
  status: ToolExecutionEntry["status"];
}

function readStringField(input: Record<string, unknown> | null, keys: string[]): string | undefined {
  if (!input) return undefined;
  for (const key of keys) {
    const value = input[key];
    if (typeof value === "string" && value.length > 0) {
      return value;
    }
  }
  return undefined;
}

function parseToolInput(raw: string): Record<string, unknown> | null {
  if (!raw || !raw.trim()) return null;
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

function detectFileChangeKind(toolName: string): FileChangeKind | null {
  const lower = (toolName || "").toLowerCase();
  if (
    lower === "write" ||
    lower === "write_file" ||
    lower === "create_file" ||
    lower.includes("file_write") ||
    lower.includes("createfile")
  ) {
    return "write";
  }
  if (
    lower === "edit" ||
    lower === "file_edit" ||
    lower === "replace_string" ||
    lower === "str_replace" ||
    lower.includes("file_edit") ||
    lower.includes("edit_file")
  ) {
    return "edit";
  }
  return null;
}

const MAX_DIFF_LINES = 800;

export function computeLineDiff(
  oldText: string,
  newText: string,
): { diff: DiffLine[]; hunkHeader: DiffHunkHeader | null } {
  const a = (oldText ?? "").split("\n");
  const b = (newText ?? "").split("\n");
  const n = a.length;
  const m = b.length;

  if (n <= 1 || m <= 1 || n > 4000 || m > 4000) {
    const out: DiffLine[] = [];
    let oldNum = 1;
    let newNum = 1;
    for (const line of a) {
      out.push({ type: "del", text: line, oldLine: oldNum++ });
    }
    for (const line of b) {
      out.push({ type: "add", text: line, newLine: newNum++ });
    }
    return {
      diff: out,
      hunkHeader: { oldStart: 1, oldLen: n, newStart: 1, newLen: m },
    };
  }

  const dp: Int32Array[] = new Array(n + 1);
  for (let i = 0; i <= n; i++) {
    dp[i] = new Int32Array(m + 1);
  }
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      if (a[i] === b[j]) {
        dp[i][j] = dp[i + 1][j + 1] + 1;
      } else {
        const down = dp[i + 1][j];
        const right = dp[i][j + 1];
        dp[i][j] = down >= right ? down : right;
      }
    }
  }

  const raw: DiffLine[] = [];
  let i = 0;
  let j = 0;
  let oldNum = 1;
  let newNum = 1;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      raw.push({ type: "context", text: a[i], oldLine: oldNum, newLine: newNum });
      i++;
      j++;
      oldNum++;
      newNum++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      raw.push({ type: "del", text: a[i], oldLine: oldNum });
      i++;
      oldNum++;
    } else {
      raw.push({ type: "add", text: b[j], newLine: newNum });
      j++;
      newNum++;
    }
  }
  while (i < n) {
    raw.push({ type: "del", text: a[i], oldLine: oldNum });
    i++;
    oldNum++;
  }
  while (j < m) {
    raw.push({ type: "add", text: b[j], newLine: newNum });
    j++;
    newNum++;
  }

  const collapsed = collapseContext(raw);
  const header: DiffHunkHeader | null = computeHunkHeader(collapsed, n, m);
  return { diff: collapsed, hunkHeader: header };
}

function computeHunkHeader(
  lines: DiffLine[],
  oldTotal: number,
  newTotal: number,
): DiffHunkHeader | null {
  if (!lines.length) return null;
  const first = lines[0];
  const firstOld = first.type === "add" ? (nextOldLine(lines) ?? 1) : first.oldLine ?? 1;
  const firstNew = first.type === "del" ? (nextNewLine(lines) ?? 1) : first.newLine ?? 1;

  let oldLen = 0;
  let newLen = 0;
  for (const line of lines) {
    if (line.type === "del") oldLen++;
    else if (line.type === "add") newLen++;
    else {
      oldLen++;
      newLen++;
    }
  }

  let oldStart = firstOld;
  let newStart = firstNew;
  if (first.type === "add") {
    oldStart = Math.max(1, oldTotal > 0 ? adjacentOldStart(lines) : firstNew);
  }
  if (first.type === "del") {
    newStart = Math.max(1, newTotal > 0 ? adjacentNewStart(lines) : firstOld);
  }

  return {
    oldStart: Math.max(1, oldStart),
    oldLen,
    newStart: Math.max(1, newStart),
    newLen,
  };
}

function nextOldLine(lines: DiffLine[]): number | undefined {
  for (const line of lines) {
    if (line.oldLine !== undefined) return line.oldLine;
  }
  return undefined;
}

function nextNewLine(lines: DiffLine[]): number | undefined {
  for (const line of lines) {
    if (line.newLine !== undefined) return line.newLine;
  }
  return undefined;
}

function adjacentOldStart(lines: DiffLine[]): number {
  const firstOld = nextOldLine(lines);
  return firstOld !== undefined ? Math.max(1, firstOld - 1) : 1;
}

function adjacentNewStart(lines: DiffLine[]): number {
  const firstNew = nextNewLine(lines);
  return firstNew !== undefined ? Math.max(1, firstNew - 1) : 1;
}

function collapseContext(lines: DiffLine[]): DiffLine[] {
  const result: DiffLine[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.type === "context") {
      let j = i;
      while (j < lines.length && lines[j].type === "context") j++;
      const run = j - i;
      const allowLeading = result.length === 0;
      const allowTrailing = j === lines.length;
      const head = allowLeading ? 1 : 0;
      const tail = allowTrailing ? 1 : 0;
      const keep = head + tail;
      if (run > keep + 4) {
        for (let k = i; k < i + head; k++) result.push(lines[k]);
        const hidden = run - keep;
        result.push({ type: "context", text: `  …（省略 ${hidden} 行相同内容）` });
        for (let k = j - tail; k < j; k++) result.push(lines[k]);
      } else {
        for (let k = i; k < j; k++) result.push(lines[k]);
      }
      i = j;
    } else {
      result.push(line);
      i++;
    }
  }
  return result;
}

function buildHunk(entry: ToolExecutionEntry): FileChangeHunk | null {
  const kind = detectFileChangeKind(entry.toolName);
  if (!kind) return null;

  const parsed = parseToolInput(entry.input);
  const filePath =
    readStringField(parsed, ["file_path", "filePath", "path", "uri"]) ?? "(unknown path)";

  const oldString = readStringField(parsed, ["oldString", "old_string", "search", "find"]);
  const newString = readStringField(parsed, ["newString", "new_string", "replace", "replacement"]);
  const content = readStringField(parsed, ["content", "new_content", "text"]);

  let diff: DiffLine[];
  let hunkHeader: DiffHunkHeader | null = null;
  let oldTotalLines = (oldString ?? "").split("\n").length;
  let newTotalLines = (newString ?? "").split("\n").length;
  let addedCount = 0;
  let removedCount = 0;

  if (kind === "edit") {
    const result = computeLineDiff(oldString ?? "", newString ?? "");
    diff = result.diff;
    hunkHeader = result.hunkHeader;
  } else {
    const text = content ?? "";
    const lines = text.split("\n").slice(0, MAX_DIFF_LINES);
    diff = lines.map((text, idx) => ({ type: "add" as const, text, newLine: idx + 1 }));
    newTotalLines = lines.length;
  }

  if (diff.length > MAX_DIFF_LINES) {
    diff = diff.slice(0, MAX_DIFF_LINES);
    diff.push({ type: "context", text: `…（已截断，剩余内容省略）` });
  }

  for (const line of diff) {
    if (line.type === "add") addedCount++;
    else if (line.type === "del") removedCount++;
  }

  return {
    toolName: entry.toolName,
    kind,
    filePath,
    oldString,
    newString,
    content,
    status: entry.status,
    diff,
    hunkHeader,
    oldTotalLines,
    newTotalLines,
    addedCount,
    removedCount,
    startedAt: entry.startedAt,
    finishedAt: entry.finishedAt,
  };
}

function priorityStatus(
  current: ToolExecutionEntry["status"],
  next: ToolExecutionEntry["status"],
): ToolExecutionEntry["status"] {
  const rank: Record<ToolExecutionEntry["status"], number> = {
    error: 3,
    running: 2,
    completed: 1,
    cancelled: 0,
  };
  return rank[next] > rank[current] ? next : current;
}

export function buildModifiedFileGroups(entries: ToolExecutionEntry[]): FileChangeGroup[] {
  const groups = new Map<string, FileChangeGroup>();

  for (const entry of entries) {
    if (entry.status === "running") continue;
    const hunk = buildHunk(entry);
    if (!hunk) continue;

    const group = groups.get(hunk.filePath);
    if (group) {
      group.hunks.push(hunk);
      group.status = priorityStatus(group.status, hunk.status);
    } else {
      groups.set(hunk.filePath, {
        filePath: hunk.filePath,
        hunks: [hunk],
        status: hunk.status,
      });
    }
  }

  return [...groups.values()];
}

export function summarizeModifiedFiles(groups: FileChangeGroup[]): {
  edited: number;
  written: number;
  added: number;
  removed: number;
} {
  let edited = 0;
  let written = 0;
  let added = 0;
  let removed = 0;
  for (const group of groups) {
    if (group.hunks.some((h) => h.kind === "write")) written++;
    else edited++;
    for (const hunk of group.hunks) {
      added += hunk.addedCount;
      removed += hunk.removedCount;
    }
  }
  return { edited, written, added, removed };
}

export function formatHunkHeader(header: DiffHunkHeader): string {
  const oldPart =
    header.oldLen === 1 ? `${header.oldStart}` : `${header.oldStart},${header.oldLen}`;
  const newPart =
    header.newLen === 1 ? `${header.newStart}` : `${header.newStart},${header.newLen}`;
  return `@@ -${oldPart} +${newPart} @@`;
}

export function shortPath(filePath: string, maxLen = 56): string {
  if (!filePath) return "(unknown path)";
  if (filePath.length <= maxLen) return filePath;
  const parts = filePath.split(/[\\/]/);
  if (parts.length <= 2) return filePath.slice(-maxLen);
  const fileName = parts[parts.length - 1];
  const dir = parts.slice(0, -1).join("/");
  const truncated = dir.length > maxLen - fileName.length - 3 ? "…" : dir.slice(0, Math.max(0, maxLen - fileName.length - 4));
  return `${truncated}/${fileName}`;
}