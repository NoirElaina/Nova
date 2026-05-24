import type { FileChangeBatch, FileChangeEntry, FileDiffLine } from "@/features/chat/services/chat-api";

export type FileHistoryEntry = {
  id: string;
  batch: FileChangeBatch;
  file: FileChangeEntry;
};

export type AggregatedFileChange = {
  id: string;
  path: string;
  absolutePath: string;
  changeType: FileChangeEntry["changeType"];
  diff: FileDiffLine[];
  firstCreatedAt: number;
  lastCreatedAt: number;
  history: FileHistoryEntry[];
};

type FileChangeAccumulator = {
  id: string;
  path: string;
  absolutePath: string;
  firstBefore?: string | null;
  latestAfter?: string | null;
  hasSnapshot: boolean;
  firstCreatedAt: number;
  lastCreatedAt: number;
  latestFile: FileChangeEntry;
  history: FileHistoryEntry[];
};

const splitLines = (value: string | null | undefined) => {
  const normalized = (value ?? "").replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  if (!normalized) return [];
  const withoutFinalBreak = normalized.endsWith("\n") ? normalized.slice(0, -1) : normalized;
  return withoutFinalBreak.split("\n");
};

const pushContextLine = (
  lines: FileDiffLine[],
  text: string,
  oldLine: number,
  newLine: number,
) => {
  lines.push({ kind: "context", oldLine, newLine, text });
};

const pushRemoveLine = (lines: FileDiffLine[], text: string, oldLine: number) => {
  lines.push({ kind: "remove", oldLine, newLine: null, text });
};

const pushAddLine = (lines: FileDiffLine[], text: string, newLine: number) => {
  lines.push({ kind: "add", oldLine: null, newLine, text });
};

const diffMiddleLines = (
  oldLines: string[],
  newLines: string[],
  oldOffset: number,
  newOffset: number,
) => {
  const result: FileDiffLine[] = [];
  if (oldLines.length * newLines.length > 200_000) {
    oldLines.forEach((line, index) => pushRemoveLine(result, line, oldOffset + index + 1));
    newLines.forEach((line, index) => pushAddLine(result, line, newOffset + index + 1));
    return result;
  }

  const lcs = Array.from({ length: oldLines.length + 1 }, () =>
    Array<number>(newLines.length + 1).fill(0),
  );
  for (let oldIndex = oldLines.length - 1; oldIndex >= 0; oldIndex -= 1) {
    for (let newIndex = newLines.length - 1; newIndex >= 0; newIndex -= 1) {
      lcs[oldIndex][newIndex] =
        oldLines[oldIndex] === newLines[newIndex]
          ? lcs[oldIndex + 1][newIndex + 1] + 1
          : Math.max(lcs[oldIndex + 1][newIndex], lcs[oldIndex][newIndex + 1]);
    }
  }

  let oldIndex = 0;
  let newIndex = 0;
  while (oldIndex < oldLines.length || newIndex < newLines.length) {
    if (
      oldIndex < oldLines.length &&
      newIndex < newLines.length &&
      oldLines[oldIndex] === newLines[newIndex]
    ) {
      pushContextLine(
        result,
        oldLines[oldIndex],
        oldOffset + oldIndex + 1,
        newOffset + newIndex + 1,
      );
      oldIndex += 1;
      newIndex += 1;
    } else if (
      newIndex < newLines.length &&
      (oldIndex >= oldLines.length || lcs[oldIndex][newIndex + 1] >= lcs[oldIndex + 1][newIndex])
    ) {
      pushAddLine(result, newLines[newIndex], newOffset + newIndex + 1);
      newIndex += 1;
    } else if (oldIndex < oldLines.length) {
      pushRemoveLine(result, oldLines[oldIndex], oldOffset + oldIndex + 1);
      oldIndex += 1;
    }
  }

  return result;
};

const diffLines = (before: string | null | undefined, after: string | null | undefined) => {
  const oldLines = splitLines(before);
  const newLines = splitLines(after);
  const result: FileDiffLine[] = [];
  let prefix = 0;
  while (
    prefix < oldLines.length &&
    prefix < newLines.length &&
    oldLines[prefix] === newLines[prefix]
  ) {
    pushContextLine(result, oldLines[prefix], prefix + 1, prefix + 1);
    prefix += 1;
  }

  let suffix = 0;
  while (
    suffix + prefix < oldLines.length &&
    suffix + prefix < newLines.length &&
    oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix += 1;
  }

  const oldMiddle = oldLines.slice(prefix, oldLines.length - suffix);
  const newMiddle = newLines.slice(prefix, newLines.length - suffix);
  result.push(...diffMiddleLines(oldMiddle, newMiddle, prefix, prefix));

  for (let index = suffix; index > 0; index -= 1) {
    const oldIndex = oldLines.length - index;
    const newIndex = newLines.length - index;
    pushContextLine(result, oldLines[oldIndex], oldIndex + 1, newIndex + 1);
  }

  return result;
};

const changeTypeFromSnapshots = (
  before: string | null | undefined,
  after: string | null | undefined,
): FileChangeEntry["changeType"] => {
  if (before === null && after !== null && after !== undefined) return "added";
  if (before !== null && before !== undefined && after === null) return "deleted";
  return "modified";
};

const hasSnapshot = (file: FileChangeEntry) =>
  Object.prototype.hasOwnProperty.call(file, "before") ||
  Object.prototype.hasOwnProperty.call(file, "after");

export const mergeFileChanges = (batches: FileChangeBatch[]): AggregatedFileChange[] => {
  const byPath = new Map<string, FileChangeAccumulator>();
  const chronologicalBatches = [...batches].reverse();

  for (const batch of chronologicalBatches) {
    if (batch.reverted) continue;
    for (const file of batch.files) {
      const id = file.absolutePath || file.path;
      const existing = byPath.get(id);
      const fileHasSnapshot = hasSnapshot(file);
      const historyEntry = { id: `${batch.id}:${id}`, batch, file };

      if (!existing) {
        byPath.set(id, {
          id,
          path: file.path,
          absolutePath: file.absolutePath,
          firstBefore: fileHasSnapshot ? file.before ?? null : undefined,
          latestAfter: fileHasSnapshot ? file.after ?? null : undefined,
          hasSnapshot: fileHasSnapshot,
          firstCreatedAt: batch.createdAt,
          lastCreatedAt: batch.createdAt,
          latestFile: file,
          history: [historyEntry],
        });
        continue;
      }

      if (fileHasSnapshot) {
        if (!existing.hasSnapshot) {
          existing.firstBefore = file.before ?? null;
          existing.hasSnapshot = true;
        }
        existing.latestAfter = file.after ?? null;
      }
      existing.lastCreatedAt = batch.createdAt;
      existing.latestFile = file;
      existing.history.push(historyEntry);
    }
  }

  return Array.from(byPath.values())
    .map((item) => {
      const diff = item.hasSnapshot
        ? diffLines(item.firstBefore, item.latestAfter)
        : item.latestFile.diff;
      return {
        id: item.id,
        path: item.path,
        absolutePath: item.absolutePath,
        changeType: item.hasSnapshot
          ? changeTypeFromSnapshots(item.firstBefore, item.latestAfter)
          : item.latestFile.changeType,
        diff,
        firstCreatedAt: item.firstCreatedAt,
        lastCreatedAt: item.lastCreatedAt,
        history: [...item.history].reverse(),
      };
    })
    .sort((a, b) => b.lastCreatedAt - a.lastCreatedAt || a.path.localeCompare(b.path));
};
