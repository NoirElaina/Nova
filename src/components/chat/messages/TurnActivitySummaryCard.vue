<script setup lang="ts">
import { computed } from "vue";
import type { ToolTurnSummary, ToolTurnEntrySnapshot } from "../../../lib/chat-types";
import {
  renderToolTurnSummaryLine,
} from "../../../features/chat/utils/tool-activity-summary";
import { summarizeToolInfo } from "../../../features/chat/utils/tool-info";
import {
  buildHunk,
  type DiffLine,
} from "../../../features/chat/utils/modified-files-summary";

const props = defineProps<{
  summary: ToolTurnSummary;
}>();

const summaryLine = computed(() => renderToolTurnSummaryLine(props.summary));

// 预计算每个条目的文件 diff hunk（仅 Write/Edit 类工具有值），
// 展开时优先展示 diff 而非 raw result JSON。
const entriesWithHunk = computed(() =>
  props.summary.entries.map((entry) => ({ entry, hunk: buildHunk(entry) })),
);

function formatToolName(name: string): string {
  const normalized = name.replace(/_/g, " ").trim();
  if (!normalized) {
    return "Tool";
  }
  return normalized.startsWith("mcp__") ? normalized.replace(/^mcp__/, "MCP ") : normalized;
}

function rowText(entry: ToolTurnEntrySnapshot): string {
  const info = summarizeToolInfo(entry.toolName, entry.input);
  return info ? `${formatToolName(entry.toolName)} ${info}` : formatToolName(entry.toolName);
}

function resultText(entry: ToolTurnEntrySnapshot): string {
  const result = entry.result.replace(/\s+$/,'');
  if (result) {
    return result;
  }
  if (entry.status === "running") {
    return "Running...";
  }
  if (entry.status === "cancelled") {
    return "Cancelled.";
  }
  return "No result.";
}

function lineClass(line: DiffLine): string {
  if (line.type === "add") return "turn-summary-card__line--add";
  if (line.type === "del") return "turn-summary-card__line--del";
  return "turn-summary-card__line--ctx";
}

function lineMarker(line: DiffLine): string {
  if (line.type === "add") return "+";
  if (line.type === "del") return "-";
  return " ";
}
</script>

<template>
  <details class="turn-summary-card">
    <summary class="turn-summary-card__summary">
      <span class="turn-summary-card__title">{{ summaryLine }}</span>
      <span class="turn-summary-card__chevron">›</span>
    </summary>

    <div class="turn-summary-card__body">
      <details
        v-for="{ entry, hunk } in entriesWithHunk"
        :key="entry.id"
        class="turn-summary-card__tool"
        :class="`turn-summary-card__tool--${entry.status}`"
      >
        <summary class="turn-summary-card__row">
          <span class="turn-summary-card__status" aria-hidden="true"></span>
          <span class="turn-summary-card__row-text">{{ rowText(entry) }}</span>
          <span class="turn-summary-card__row-chevron">›</span>
        </summary>
        <pre
          v-if="hunk"
          class="turn-summary-card__diff"
        ><code><span
          v-for="(line, lineIdx) in hunk.diff"
          :key="lineIdx"
          :class="lineClass(line)"
          class="turn-summary-card__line"
        ><span class="turn-summary-card__marker">{{ lineMarker(line) }}</span><span class="turn-summary-card__text">{{ line.text || ' ' }}</span></span></code></pre>
        <pre v-else class="turn-summary-card__result">{{ resultText(entry) }}</pre>
      </details>
    </div>
  </details>
</template>

<style scoped>
.turn-summary-card {
  margin: 0;
  color: #6b7280;
  interpolate-size: allow-keywords;
}

.turn-summary-card[open] {
  color: #4b5563;
}

.turn-summary-card__summary {
  list-style: none;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 100%;
  font-size: 15px;
  line-height: 1.45;
  color: inherit;
}

.turn-summary-card__summary::-webkit-details-marker {
  display: none;
}

.turn-summary-card__title {
  min-width: 0;
}

.turn-summary-card__chevron {
  font-size: 18px;
  line-height: 1;
  transition: transform 0.16s ease;
}

.turn-summary-card[open] .turn-summary-card__chevron {
  transform: rotate(90deg);
}

.turn-summary-card::details-content {
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

.turn-summary-card[open]::details-content {
  block-size: auto;
  opacity: 1;
  transform: translateY(0);
}

.turn-summary-card__body {
  margin-top: 8px;
  border: 1px solid #e5e7eb;
  background: #fff;
  border-radius: 8px;
  padding: 10px 14px;
}

.turn-summary-card__tool {
  margin: 0;
  interpolate-size: allow-keywords;
}

.turn-summary-card__tool + .turn-summary-card__tool {
  margin-top: 7px;
}

.turn-summary-card__row {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  max-width: 100%;
  min-height: 22px;
  cursor: pointer;
  list-style: none;
  font-size: 15px;
  line-height: 1.35;
  color: #6b7280;
}

.turn-summary-card__row::-webkit-details-marker {
  display: none;
}

.turn-summary-card__status {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: #9ca3af;
  flex: 0 0 auto;
}

.turn-summary-card__tool--running .turn-summary-card__status {
  background: #2563eb;
  animation: status-pulse 1.1s ease-in-out infinite;
}

.turn-summary-card__tool--completed .turn-summary-card__status {
  background: #16a34a;
}

.turn-summary-card__tool--error .turn-summary-card__status {
  background: #dc2626;
}

.turn-summary-card__tool--cancelled .turn-summary-card__status {
  background: #9ca3af;
}

.turn-summary-card__tool--error .turn-summary-card__row {
  color: #dc2626;
}

.turn-summary-card__row-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.turn-summary-card__row-chevron {
  color: #9ca3af;
  font-size: 18px;
  line-height: 1;
  transition: transform 0.16s ease;
}

.turn-summary-card__tool[open] .turn-summary-card__row-chevron {
  transform: rotate(90deg);
}

.turn-summary-card__tool::details-content {
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

.turn-summary-card__tool[open]::details-content {
  block-size: auto;
  opacity: 1;
  transform: translateY(0);
}

.turn-summary-card__result {
  margin: 7px 0 0 14px;
  max-height: 260px;
  overflow: auto;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  background: #fafafa;
  padding: 8px 10px;
  color: #374151;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 12px;
  line-height: 1.5;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
}

.turn-summary-card__diff {
  margin: 7px 0 0 14px;
  max-height: 320px;
  overflow: auto;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  background: #fafafa;
  padding: 6px 0;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  font-size: 12px;
  line-height: 1.55;
}

.turn-summary-card__diff code {
  display: block;
  white-space: pre;
}

.turn-summary-card__line {
  display: flex;
  align-items: baseline;
  min-height: 18px;
  padding: 0 10px 0 0;
  white-space: pre;
}

.turn-summary-card__marker {
  flex: 0 0 auto;
  width: 1.4ch;
  text-align: center;
  font-weight: 600;
  padding-left: 6px;
}

.turn-summary-card__text {
  flex: 1 1 auto;
  min-width: 0;
  white-space: pre-wrap;
  word-break: break-word;
  padding-left: 4px;
}

.turn-summary-card__line--add {
  background: rgba(34, 197, 94, 0.12);
  color: #166534;
}

.turn-summary-card__line--add .turn-summary-card__marker {
  color: #16a34a;
}

.turn-summary-card__line--del {
  background: rgba(239, 68, 68, 0.12);
  color: #991b1b;
}

.turn-summary-card__line--del .turn-summary-card__marker {
  color: #dc2626;
}

.turn-summary-card__line--ctx {
  color: #94a3b8;
}

.turn-summary-card__diff::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

.turn-summary-card__diff::-webkit-scrollbar-track {
  background: transparent;
}

.turn-summary-card__diff::-webkit-scrollbar-thumb {
  background: rgba(148, 163, 184, 0.35);
  border-radius: 999px;
}

.turn-summary-card__tool--error .turn-summary-card__result {
  border-color: #fecaca;
  background: #fff7f7;
  color: #b91c1c;
}

.dark .turn-summary-card {
  color: #a3a3a3;
}

.dark .turn-summary-card[open] {
  color: #d4d4d4;
}

.dark .turn-summary-card__body {
  border-color: #3f4652;
  background: #1f2937;
}

.dark .turn-summary-card__row {
  color: #a3a3a3;
}

.dark .turn-summary-card__tool--error .turn-summary-card__row {
  color: #f87171;
}

.dark .turn-summary-card__result {
  border-color: #3f4652;
  background: #111827;
  color: #d1d5db;
}

.dark .turn-summary-card__diff {
  border-color: #3f4652;
  background: #111827;
}

.dark .turn-summary-card__line--add {
  background: rgba(34, 197, 94, 0.18);
  color: #86efac;
}

.dark .turn-summary-card__line--add .turn-summary-card__marker {
  color: #4ade80;
}

.dark .turn-summary-card__line--del {
  background: rgba(239, 68, 68, 0.18);
  color: #fca5a5;
}

.dark .turn-summary-card__line--del .turn-summary-card__marker {
  color: #f87171;
}

.dark .turn-summary-card__line--ctx {
  color: #6b7280;
}

.dark .turn-summary-card__tool--error .turn-summary-card__result {
  border-color: #7f1d1d;
  background: #1f1315;
  color: #fca5a5;
}

@keyframes status-pulse {
  0%,
  100% {
    opacity: 0.4;
  }

  50% {
    opacity: 1;
  }
}
</style>
