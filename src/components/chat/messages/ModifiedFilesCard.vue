<script setup lang="ts">
import { computed } from "vue";
import type { ToolExecutionEntry } from "../../../lib/chat-types";
import {
  buildModifiedFileGroups,
  summarizeModifiedFiles,
  shortPath,
  formatHunkHeader,
  type DiffLine,
  type FileChangeHunk,
} from "../../../features/chat/utils/modified-files-summary";

const props = defineProps<{
  entries: ToolExecutionEntry[];
}>();

const groups = computed(() => buildModifiedFileGroups(props.entries));
const counts = computed(() => summarizeModifiedFiles(groups.value));
const totalFiles = computed(() => groups.value.length);

const headerText = computed(() => {
  const parts: string[] = [];
  if (counts.value.edited > 0) parts.push(`编辑 ${counts.value.edited}`);
  if (counts.value.written > 0) parts.push(`创建/覆盖 ${counts.value.written}`);
  if (counts.value.added > 0 || counts.value.removed > 0) {
    parts.push(`+${counts.value.added} -${counts.value.removed}`);
  }
  return parts.length ? `已修改 ${totalFiles.value} 个文件 · ${parts.join(" · ")}` : `已修改 ${totalFiles.value} 个文件`;
});

function hunkLabel(hunk: FileChangeHunk): string {
  if (hunk.kind === "write") return "创建/覆盖";
  return "编辑";
}

function hunkStats(hunk: FileChangeHunk): string {
  return `+${hunk.addedCount} -${hunk.removedCount}`;
}

function hunkHeaderText(hunk: FileChangeHunk): string | null {
  if (hunk.hunkHeader) return formatHunkHeader(hunk.hunkHeader);
  if (hunk.kind === "write" && hunk.newTotalLines > 0) {
    return `@@ +1,${hunk.newTotalLines} @@ (:new file)`;
  }
  return null;
}

function lineClass(line: DiffLine): string {
  if (line.type === "add") return "modified-files__line--add";
  if (line.type === "del") return "modified-files__line--del";
  return "modified-files__line--ctx";
}

function lineMarker(line: DiffLine): string {
  if (line.type === "add") return "+";
  if (line.type === "del") return "-";
  return " ";
}

function oldLineNumber(line: DiffLine): string {
  return line.oldLine !== undefined ? String(line.oldLine) : "";
}

function newLineNumber(line: DiffLine): string {
  return line.newLine !== undefined ? String(line.newLine) : "";
}

function groupClass(group: { status: ToolExecutionEntry["status"] }): string {
  return `modified-files__file--${group.status}`;
}
</script>

<template>
  <details v-if="totalFiles > 0" class="modified-files-card">
    <summary class="modified-files-card__summary">
      <span class="modified-files-card__dot" aria-hidden="true"></span>
      <span class="modified-files-card__title">{{ headerText }}</span>
      <span class="modified-files-card__chevron">›</span>
    </summary>

    <div class="modified-files-card__body">
      <details
        v-for="group in groups"
        :key="group.filePath"
        class="modified-files__file"
        :class="groupClass(group)"
      >
        <summary class="modified-files__file-row">
          <svg class="modified-files__file-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
          <span class="modified-files__file-path" :title="group.filePath">{{ shortPath(group.filePath) }}</span>
          <span class="modified-files__file-count" aria-hidden="true">{{ group.hunks.length }}</span>
          <span class="modified-files__file-chevron">›</span>
        </summary>

        <div class="modified-files__hunks">
          <div
            v-for="(hunk, idx) in group.hunks"
            :key="idx"
            class="modified-files__hunk"
          >
            <div class="modified-files__hunk-meta">
              <span class="modified-files__hunk-tag">{{ hunkLabel(hunk) }}</span>
              <span class="modified-files__hunk-tool">{{ hunk.toolName }}</span>
              <span class="modified-files__hunk-stats">{{ hunkStats(hunk) }}</span>
              <span
                v-if="hunk.status === 'error'"
                class="modified-files__hunk-status modified-files__hunk-status--error"
              >失败</span>
              <span
                v-else-if="hunk.status === 'cancelled'"
                class="modified-files__hunk-status modified-files__hunk-status--cancelled"
              >已取消</span>
              <span
                v-else-if="hunk.status === 'running'"
                class="modified-files__hunk-status modified-files__hunk-status--running"
              >进行中</span>
            </div>
            <pre
              v-if="hunkHeaderText(hunk)"
              class="modified-files__hunk-header"
            >{{ hunkHeaderText(hunk) }}</pre>
            <pre class="modified-files__diff"><code><span
              v-for="(line, lineIdx) in hunk.diff"
              :key="lineIdx"
              :class="lineClass(line)"
              class="modified-files__line"
            ><span class="modified-files__ln modified-files__ln--old">{{ oldLineNumber(line) }}</span><span class="modified-files__ln modified-files__ln--new">{{ newLineNumber(line) }}</span><span class="modified-files__marker">{{ lineMarker(line) }}</span><span class="modified-files__text">{{ line.text || ' ' }}</span></span></code></pre>
          </div>
        </div>
      </details>
    </div>
  </details>
</template>

<style scoped>
.modified-files-card {
  margin: 0;
  color: #6b7280;
  interpolate-size: allow-keywords;
}

.modified-files-card[open] {
  color: #4b5563;
}

.modified-files-card__summary {
  list-style: none;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  font-size: 15px;
  line-height: 1.45;
  color: inherit;
}

.modified-files-card__summary::-webkit-details-marker {
  display: none;
}

.modified-files-card__dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: #6366f1;
  flex: 0 0 auto;
}

.modified-files-card__title {
  min-width: 0;
}

.modified-files-card__chevron {
  font-size: 18px;
  line-height: 1;
  transition: transform 0.16s ease;
}

.modified-files-card[open] .modified-files-card__chevron {
  transform: rotate(90deg);
}

.modified-files-card::details-content {
  block-size: 0;
  opacity: 0;
  overflow: hidden;
  transform: translateY(-4px);
  transition:
    block-size 0.22s ease,
    opacity 0.16s ease,
    transform 0.22s ease,
    content-visibility 0.22s ease allow-discrete;
}

.modified-files-card[open]::details-content {
  block-size: auto;
  opacity: 1;
  transform: translateY(0);
}

.modified-files-card__body {
  margin-top: 8px;
  border: 1px solid #e5e7eb;
  background: #fff;
  border-radius: 8px;
  padding: 8px 10px;
}

.modified-files__file {
  margin: 0;
  interpolate-size: allow-keywords;
}

.modified-files__file + .modified-files__file {
  margin-top: 6px;
}

.modified-files__file--error .modified-files__file-row {
  color: #b91c1c;
}

.modified-files__file--completed .modified-files__file-path {
  color: #1f2937;
}

.modified-files__file-row {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  min-height: 22px;
  cursor: pointer;
  list-style: none;
  font-size: 14px;
  line-height: 1.35;
  color: #6b7280;
}

.modified-files__file-row::-webkit-details-marker {
  display: none;
}

.modified-files__file-icon {
  flex: 0 0 auto;
  color: #9ca3af;
}

.modified-files__file-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  font-size: 13px;
}

.modified-files__file-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 999px;
  background: #eef2ff;
  color: #4f46e5;
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}

.modified-files__file-chevron {
  color: #9ca3af;
  font-size: 18px;
  line-height: 1;
  transition: transform 0.16s ease;
}

.modified-files__file[open] .modified-files__file-chevron {
  transform: rotate(90deg);
}

.modified-files__file::details-content {
  block-size: 0;
  opacity: 0;
  overflow: hidden;
  transform: translateY(-3px);
  transition:
    block-size 0.2s ease,
    opacity 0.14s ease,
    transform 0.2s ease,
    content-visibility 0.2s ease allow-discrete;
}

.modified-files__file[open]::details-content {
  block-size: auto;
  opacity: 1;
  transform: translateY(0);
}

.modified-files__hunks {
  margin: 8px 0 4px 20px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.modified-files__hunk {
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  background: #fafafa;
  overflow: hidden;
}

.modified-files__hunk-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border-bottom: 1px solid #eef0f3;
  background: #fff;
  font-size: 11px;
  color: #6b7280;
}

.modified-files__hunk-tag {
  display: inline-flex;
  align-items: center;
  padding: 1px 6px;
  border-radius: 4px;
  background: #ecfeff;
  color: #0e7490;
  border: 1px solid #cffafe;
  font-weight: 600;
  letter-spacing: 0.04em;
}

.modified-files__hunk-tool {
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  color: #9ca3af;
}

.modified-files__hunk-stats {
  margin-left: auto;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  font-variant-numeric: tabular-nums;
  color: #6b7280;
}

.modified-files__hunk-status {
  margin-left: 6px;
}

.modified-files__hunk-header {
  margin: 0;
  padding: 4px 10px;
  background: #f1f5f9;
  color: #6b7280;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  font-size: 11px;
  line-height: 1.5;
  border-bottom: 1px solid #eef0f3;
  white-space: pre;
  overflow-x: auto;
}

.modified-files__hunk-status--error {
  color: #dc2626;
}

.modified-files__hunk-status--cancelled {
  color: #9ca3af;
}

.modified-files__hunk-status--running {
  color: #2563eb;
}

.modified-files__diff {
  margin: 0;
  padding: 6px 0;
  max-height: 280px;
  overflow: auto;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  font-size: 12px;
  line-height: 1.55;
}

.modified-files__diff code {
  display: block;
  white-space: pre;
}

.modified-files__line {
  display: flex;
  align-items: baseline;
  min-height: 18px;
  padding: 0 8px 0 0;
  white-space: pre;
}

.modified-files__ln {
  flex: 0 0 auto;
  min-width: 3ch;
  padding: 0 6px 0 4px;
  text-align: right;
  color: #94a3b8;
  font-variant-numeric: tabular-nums;
  user-select: none;
  opacity: 0.7;
}

.modified-files__ln--old {
  border-right: 1px solid rgba(148, 163, 184, 0.18);
}

.modified-files__ln--new {
  border-right: 1px solid rgba(148, 163, 184, 0.18);
}

.modified-files__marker {
  flex: 0 0 auto;
  width: 1.2ch;
  text-align: center;
  font-weight: 600;
}

.modified-files__text {
  flex: 1 1 auto;
  min-width: 0;
  white-space: pre-wrap;
  word-break: break-word;
  padding-left: 4px;
}

.modified-files__line--add {
  background: rgba(34, 197, 94, 0.12);
  color: #166534;
}

.modified-files__line--add .modified-files__marker {
  color: #16a34a;
}

.modified-files__line--del {
  background: rgba(239, 68, 68, 0.12);
  color: #991b1b;
}

.modified-files__line--del .modified-files__marker {
  color: #dc2626;
}

.modified-files__line--ctx {
  color: #94a3b8;
}

.modified-files__line--add .modified-files__ln,
.modified-files__line--del .modified-files__ln {
  opacity: 1;
}

.dark .modified-files-card {
  color: #a3a3a3;
}

.dark .modified-files-card[open] {
  color: #d4d4d4;
}

.dark .modified-files-card__body {
  border-color: #3f4652;
  background: #1f2937;
}

.dark .modified-files__file-row {
  color: #a3a3a3;
}

.dark .modified-files__file--completed .modified-files__file-path {
  color: #e5e7eb;
}

.dark .modified-files__file--error .modified-files__file-row {
  color: #f87171;
}

.dark .modified-files__file-icon {
  color: #6b7280;
}

.dark .modified-files__file-count {
  background: rgba(79, 70, 229, 0.18);
  color: #c7d2fe;
}

.dark .modified-files__file-chevron {
  color: #6b7280;
}

.dark .modified-files__hunk {
  border-color: #374151;
  background: #111827;
}

.dark .modified-files__hunk-meta {
  border-bottom-color: #374151;
  background: #1f2937;
  color: #a3a3a3;
}

.dark .modified-files__hunk-stats {
  color: #9ca3af;
}

.dark .modified-files__hunk-header {
  background: #111827;
  color: #9ca3af;
  border-bottom-color: #374151;
}

.dark .modified-files__hunk-tag {
  background: rgba(8, 145, 178, 0.18);
  color: #67e8f9;
  border-color: rgba(8, 145, 178, 0.35);
}

.dark .modified-files__hunk-tool {
  color: #6b7280;
}

.dark .modified-files__hunk-status--error {
  color: #fca5a5;
}

.dark .modified-files__hunk-status--cancelled {
  color: #9ca3af;
}

.dark .modified-files__hunk-status--running {
  color: #93c5fd;
}

.dark .modified-files__line--add {
  background: rgba(34, 197, 94, 0.18);
  color: #86efac;
}

.dark .modified-files__line--add .modified-files__marker {
  color: #4ade80;
}

.dark .modified-files__line--del {
  background: rgba(239, 68, 68, 0.18);
  color: #fca5a5;
}

.dark .modified-files__line--del .modified-files__marker {
  color: #f87171;
}

.dark .modified-files__line--ctx {
  color: #6b7280;
}

.dark .modified-files__ln {
  color: #6b7280;
  border-right-color: rgba(107, 114, 128, 0.32);
}

.modified-files__diff::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

.modified-files__diff::-webkit-scrollbar-track {
  background: transparent;
}

.modified-files__diff::-webkit-scrollbar-thumb {
  background: rgba(148, 163, 184, 0.35);
  border-radius: 999px;
}
</style>