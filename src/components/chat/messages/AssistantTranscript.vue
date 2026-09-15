<script setup lang="ts">
import { computed } from "vue";
import type {
  AssistantTranscriptSegment,
  ToolExecutionEntry,
  ToolTurnSummary,
} from "../../../lib/chat-types";
import { buildToolSummaryForSegment } from "../../../features/chat/utils/assistant-transcript";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import TurnActivitySummaryCard from "./TurnActivitySummaryCard.vue";
import ModifiedFilesCard from "./ModifiedFilesCard.vue";

const props = withDefaults(
  defineProps<{
    segments: AssistantTranscriptSegment[];
    entries?: ToolExecutionEntry[];
    toolSummary?: ToolTurnSummary;
    /** 流式 live turn：仅最后一个 text/reasoning 段走 live 渲染 */
    live?: boolean;
  }>(),
  { live: false },
);

// 不深拷贝：直接过滤，降低每 token 分配
const renderSegments = computed(() =>
  props.segments.filter((segment) => {
    if (segment.type === "reasoning") {
      return segment.text.trim().length > 0;
    }
    if (segment.type === "tools") {
      return segment.toolIds.length > 0;
    }
    return segment.text.trim().length > 0;
  }),
);

const aggregatedToolEntries = computed<ToolExecutionEntry[]>(() => {
  const byId = new Map<string, ToolExecutionEntry>();
  for (const entry of props.entries ?? []) {
    byId.set(entry.id, entry);
  }
  if (props.toolSummary) {
    for (const entry of props.toolSummary.entries) {
      if (!byId.has(entry.id)) {
        byId.set(entry.id, entry);
      }
    }
  }
  return [...byId.values()];
});

function segmentKey(segment: AssistantTranscriptSegment, index: number): string {
  if (segment.type === "tools") {
    // 锚定该组第一个工具 ID（追加新工具时不变）。
    // 不能混入 index/length：流式执行中组内追加工具会让 key 变化，
    // 卡片被销毁重建，<details> 的展开状态随之丢失（面板莫名折叠）。
    return `tools-${segment.toolIds[0] ?? index}`;
  }
  return `${index}-${segment.type}`;
}

function isLiveSegment(segment: AssistantTranscriptSegment, index: number): boolean {
  if (!props.live) return false;
  if (segment.type === "tools") return false;
  // 仅最后一个 text/reasoning 段实时重渲染；前面的闭合段走缓存
  const segs = renderSegments.value;
  for (let i = segs.length - 1; i >= 0; i -= 1) {
    const s = segs[i];
    if (s.type === "text" || s.type === "reasoning") {
      return i === index;
    }
  }
  return false;
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

/** 流式思考预览：取最后一行非空文字的尾部（右对齐展示，溢出由 CSS 左侧渐隐收掉）。 */
function reasoningLivePreview(text: string): string {
  const lines = text.split(/\r?\n/);
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const line = lines[i].trim();
    if (line) {
      return line.length > 200 ? line.slice(-200) : line;
    }
  }
  return "";
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
        :live="isLiveSegment(segment, index)"
      />

      <details
        v-else-if="segment.type === 'reasoning'"
        class="transcript-reasoning"
      >
        <summary class="transcript-reasoning__summary">
          <span class="transcript-reasoning__title">{{ reasoningSummary(segment.text) }}</span>
          <span class="transcript-reasoning__chevron">›</span>
          <span
            v-if="isLiveSegment(segment, index) && reasoningLivePreview(segment.text)"
            class="transcript-reasoning__live"
          ><span class="transcript-reasoning__live-text">{{ reasoningLivePreview(segment.text) }}</span></span>
        </summary>
        <div class="transcript-reasoning__body">
          <MarkdownRenderer
            :content="segment.text"
            :live="isLiveSegment(segment, index)"
          />
        </div>
      </details>

      <TurnActivitySummaryCard
        v-else-if="toolSegmentSummary(segment)"
        :summary="toolSegmentSummary(segment)!"
      />
    </template>

    <ModifiedFilesCard
      v-if="aggregatedToolEntries.length > 0"
      :entries="aggregatedToolEntries"
    />
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
  display: flex;
  flex-wrap: wrap;
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

/* 流式思考预览：单行最新思考文字 + 流光扫过动画；展开详情或思考结束后隐藏。
   direction:rtl + text-align:left：短行左对齐，长行从左侧裁掉（最新内容恒可见），
   左缘用遮罩渐隐代替省略号（文字透明，CSS 省略号不可见）。 */
.transcript-reasoning__live {
  flex-basis: 100%;
  display: block;
  direction: rtl;
  text-align: left;
  overflow: hidden;
  white-space: nowrap;
  margin-top: 1px;
  font-size: 12.5px;
  line-height: 1.5;
  -webkit-mask-image: linear-gradient(90deg, transparent 0, #000 32px);
  mask-image: linear-gradient(90deg, transparent 0, #000 32px);
}

.transcript-reasoning__live-text {
  unicode-bidi: plaintext;
  background: linear-gradient(
    90deg,
    #9aa3af 0%,
    #9aa3af 35%,
    #1f2937 50%,
    #9aa3af 65%,
    #9aa3af 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  animation: reasoning-shimmer 1.8s linear infinite;
}

.transcript-reasoning[open] .transcript-reasoning__live {
  display: none;
}

@keyframes reasoning-shimmer {
  from {
    background-position: 200% 0;
  }
  to {
    background-position: -200% 0;
  }
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

.dark .transcript-reasoning__live-text {
  background: linear-gradient(
    90deg,
    #8a8f98 0%,
    #8a8f98 35%,
    #f3f4f6 50%,
    #8a8f98 65%,
    #8a8f98 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
}

.dark .transcript-reasoning__body {
  border-color: #3f4652;
  background: #1f2937;
}

.dark .transcript-reasoning__body::-webkit-scrollbar-thumb {
  background: rgba(163, 163, 163, 0.25);
}
</style>
