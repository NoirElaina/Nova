<script setup lang="ts">
import { computed } from "vue";
import type { ToolExecutionEntry, ToolTurnSummary } from "../../../lib/chat-types";
import {
  renderToolTurnCategoryLine,
  renderToolTurnSummaryLine,
} from "../../../features/chat/utils/tool-activity-summary";
import CurrentTurnActivityRail from "./CurrentTurnActivityRail.vue";

const props = defineProps<{
  summary: ToolTurnSummary;
}>();

const summaryLine = computed(() => renderToolTurnSummaryLine(props.summary));
const categoryLine = computed(() => renderToolTurnCategoryLine(props.summary));
const detailEntries = computed<ToolExecutionEntry[]>(() => props.summary.entries.map((entry) => ({ ...entry })));
</script>

<template>
  <details class="turn-summary-card">
    <summary class="turn-summary-card__summary">
      <div class="turn-summary-card__header">
        <div class="turn-summary-card__title">{{ summaryLine }}</div>
        <div class="turn-summary-card__meta">{{ categoryLine }}</div>
      </div>
      <span class="turn-summary-card__toggle">点开看详情</span>
    </summary>

    <div class="turn-summary-card__body">
      <CurrentTurnActivityRail :entries="detailEntries" />
    </div>
  </details>
</template>

<style scoped>
.turn-summary-card {
  margin: 10px 0 12px;
  border: 1px solid #e5e7eb;
  background: #f3f3f3;
  border-radius: 14px;
  overflow: hidden;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.035);
}

.turn-summary-card__summary {
  list-style: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
}

.turn-summary-card__summary::-webkit-details-marker {
  display: none;
}

.turn-summary-card__header {
  min-width: 0;
}

.turn-summary-card__title {
  font-size: 13px;
  line-height: 1.25;
  font-weight: 700;
  color: #111827;
}

.turn-summary-card__meta {
  margin-top: 3px;
  font-size: 11px;
  line-height: 1.4;
  color: #64748b;
}

.turn-summary-card__toggle {
  flex: 0 0 auto;
  font-size: 11px;
  color: #64748b;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.turn-summary-card__toggle::before {
  content: "▸";
  transition: transform 0.18s ease;
}

.turn-summary-card[open] .turn-summary-card__toggle::before {
  transform: rotate(90deg);
}

.turn-summary-card__body {
  padding: 0 14px 12px;
}

.dark .turn-summary-card {
  border-color: rgba(59, 130, 246, 0.38);
  background:
    linear-gradient(135deg, rgba(30, 58, 138, 0.24), rgba(24, 31, 42, 0.96) 42%),
    #1f2937;
}

.dark .turn-summary-card__title {
  color: #f8fafc;
}

.dark .turn-summary-card__meta {
  color: #cbd5e1;
}

.dark .turn-summary-card__toggle {
  color: #94a3b8;
}
</style>
