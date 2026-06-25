<script setup lang="ts">
import { computed } from "vue";
import type {
  AssistantTranscriptSegment,
  ToolExecutionEntry,
  ToolTurnSummary,
} from "../../../lib/chat-types";
import {
  buildToolSummaryForSegment,
  cloneTranscriptSegments,
} from "../../../features/chat/utils/assistant-transcript";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import TurnActivitySummaryCard from "./TurnActivitySummaryCard.vue";

const props = defineProps<{
  segments: AssistantTranscriptSegment[];
  entries?: ToolExecutionEntry[];
  toolSummary?: ToolTurnSummary;
}>();

const renderSegments = computed(() =>
  cloneTranscriptSegments(props.segments).filter((segment) => {
    if (segment.type === "reasoning") {
      return segment.text.trim().length > 0;
    }
    if (segment.type === "tools") {
      return segment.toolIds.length > 0;
    }
    return segment.text.trim().length > 0;
  }),
);

function segmentKey(segment: AssistantTranscriptSegment, index: number): string {
  if (segment.type === "tools") {
    return `${index}-tools-${segment.toolIds.join("-")}`;
  }
  return `${index}-${segment.type}`;
}

function toolSegmentSummary(segment: Extract<AssistantTranscriptSegment, { type: "tools" }>) {
  return buildToolSummaryForSegment(segment, props.entries ?? [], props.toolSummary);
}

function reasoningSummary(text: string): string {
  const chars = text.trim().length;
  if (chars <= 0) {
    return "Thinking";
  }
  return chars < 1000 ? `Thinking · ${chars} chars` : `Thinking · ${(chars / 1000).toFixed(1)}k chars`;
}
</script>

<template>
  <div class="assistant-transcript">
    <template
      v-for="(segment, index) in renderSegments"
      :key="segmentKey(segment, index)"
    >
      <MarkdownRenderer
        v-if="segment.type === 'text'"
        :content="segment.text"
      />

      <details
        v-else-if="segment.type === 'reasoning'"
        class="transcript-reasoning"
      >
        <summary class="transcript-reasoning__summary">
          <span class="transcript-reasoning__title">{{ reasoningSummary(segment.text) }}</span>
          <span class="transcript-reasoning__chevron">›</span>
        </summary>
        <div class="transcript-reasoning__body">
          <MarkdownRenderer :content="segment.text" />
        </div>
      </details>

      <TurnActivitySummaryCard
        v-else-if="toolSegmentSummary(segment)"
        :summary="toolSegmentSummary(segment)!"
      />
    </template>
  </div>
</template>

<style scoped>
.assistant-transcript {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.transcript-reasoning {
  margin: 0;
  color: #6b7280;
  interpolate-size: allow-keywords;
}

.transcript-reasoning[open] {
  color: #4b5563;
}

.transcript-reasoning__summary {
  cursor: pointer;
  list-style: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 100%;
  font-size: 15px;
  line-height: 1.45;
  color: inherit;
}

.transcript-reasoning__summary::-webkit-details-marker {
  display: none;
}

.transcript-reasoning__title {
  min-width: 0;
}

.transcript-reasoning__chevron {
  font-size: 18px;
  line-height: 1;
  transition: transform 0.16s ease;
}

.transcript-reasoning[open] .transcript-reasoning__chevron {
  transform: rotate(90deg);
}

.transcript-reasoning::details-content {
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

.transcript-reasoning[open]::details-content {
  block-size: auto;
  opacity: 1;
  transform: translateY(0);
}

.transcript-reasoning__body {
  margin-top: 8px;
  max-height: 280px;
  overflow-y: auto;
  border: 1px solid #e5e7eb;
  background: #fff;
  border-radius: 8px;
  padding: 10px 12px;
}

.transcript-reasoning__body::-webkit-scrollbar {
  width: 4px;
}

.transcript-reasoning__body::-webkit-scrollbar-track {
  background: transparent;
}

.transcript-reasoning__body::-webkit-scrollbar-thumb {
  background: rgba(107, 114, 128, 0.28);
  border-radius: 999px;
}

.dark .transcript-reasoning {
  color: #a3a3a3;
}

.dark .transcript-reasoning[open] {
  color: #d4d4d4;
}

.dark .transcript-reasoning__body {
  border-color: #3f4652;
  background: #1f2937;
}

.dark .transcript-reasoning__body::-webkit-scrollbar-thumb {
  background: rgba(163, 163, 163, 0.25);
}
</style>
