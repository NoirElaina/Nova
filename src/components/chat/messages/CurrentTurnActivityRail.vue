<script setup lang="ts">
import { computed } from "vue";
import type { ToolExecutionEntry } from "../../../lib/chat-types";
import { summarizeToolInfo } from "../../../features/chat/utils/tool-info";

const props = defineProps<{
  entries: ToolExecutionEntry[];
  waitKind?: "permission" | "question" | null;
}>();

type ActivityChip = {
  id: string;
  title: string;
  detail: string;
  status: ToolExecutionEntry["status"] | "waiting";
};

const statusLabelMap: Record<ActivityChip["status"], string> = {
  running: "执行中",
  completed: "已完成",
  error: "执行失败",
  cancelled: "已取消",
  waiting: "等待中",
};

function formatToolTitle(toolName: string): string {
  if (!toolName) return "工具";
  if (toolName.startsWith("mcp__")) {
    return toolName.replace(/^mcp__/, "MCP · ");
  }
  return toolName.replace(/_/g, " ");
}

function buildDetail(entry: ToolExecutionEntry): string {
  const inputSummary = summarizeToolInfo(entry.toolName, entry.input);
  if (inputSummary) {
    return inputSummary;
  }

  const text = (entry.result || "").trim();
  if (!text) {
    return statusLabelMap[entry.status];
  }

  return text.length > 72 ? `${text.slice(0, 72)}...` : text;
}

const toolChips = computed<ActivityChip[]>(() =>
  props.entries.map((entry) => ({
    id: entry.id,
    title: formatToolTitle(entry.toolName),
    detail: buildDetail(entry),
    status: entry.status,
  })),
);

const waitChip = computed<ActivityChip | null>(() => {
  if (!props.waitKind) {
    return null;
  }

  return {
    id: "wait-state",
    title: props.waitKind === "permission" ? "等待权限确认" : "等待你的输入",
    detail:
      props.waitKind === "permission"
        ? "需要你先确认，本轮才会继续。"
        : "Nova 需要你的回答后继续执行。",
    status: "waiting",
  };
});

const chips = computed<ActivityChip[]>(() =>
  waitChip.value ? [...toolChips.value, waitChip.value] : toolChips.value,
);
</script>

<template>
  <TransitionGroup name="activity-chip" tag="div" class="activity-rail">
    <div
      v-for="chip in chips"
      :key="chip.id"
      class="activity-chip"
      :class="`activity-chip--${chip.status}`"
    >
      <div class="activity-chip__icon" aria-hidden="true">
        <span v-if="chip.status === 'running'" class="activity-spinner"></span>
        <svg
          v-else-if="chip.status === 'completed'"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
        <svg
          v-else-if="chip.status === 'error'"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="15" y1="9" x2="9" y2="15" />
          <line x1="9" y1="9" x2="15" y2="15" />
        </svg>
        <svg
          v-else-if="chip.status === 'cancelled'"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="10" />
          <line x1="8" y1="12" x2="16" y2="12" />
        </svg>
        <span v-else class="activity-pulse"></span>
      </div>

      <div class="activity-chip__body">
        <div class="activity-chip__title">{{ chip.title }}</div>
        <div class="activity-chip__detail">{{ chip.detail }}</div>
      </div>
    </div>
  </TransitionGroup>
</template>

<style scoped>
.activity-rail {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 10px 0 12px;
}

.activity-chip {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
  max-width: 100%;
  border-radius: 14px;
  padding: 9px 11px;
  border: 1px solid #dbeafe;
  background:
    linear-gradient(135deg, rgba(239, 246, 255, 0.62), rgba(255, 255, 255, 0.98) 48%),
    #ffffff;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.035);
}

.activity-chip__icon {
  width: 22px;
  height: 22px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 999px;
}

.activity-chip__body {
  min-width: 0;
}

.activity-chip__title {
  font-size: 12px;
  line-height: 1.2;
  font-weight: 600;
  color: #111827;
}

.activity-chip__detail {
  margin-top: 2px;
  font-size: 11px;
  line-height: 1.35;
  color: #64748b;
  word-break: break-word;
}

.activity-chip--running {
  border-color: #bfdbfe;
  background:
    linear-gradient(135deg, rgba(239, 246, 255, 0.98), rgba(219, 234, 254, 0.86)),
    #eff6ff;
}

.activity-chip--running .activity-chip__icon {
  color: #2563eb;
  background: rgba(191, 219, 254, 0.86);
}

.activity-chip--completed {
  border-color: #cde3d5;
  background:
    linear-gradient(135deg, rgba(241, 252, 245, 0.96), rgba(228, 247, 235, 0.94)),
    #edf9f1;
}

.activity-chip--completed .activity-chip__icon {
  color: #2d7353;
  background: rgba(191, 226, 203, 0.8);
}

.activity-chip--error {
  border-color: #efcbc7;
  background:
    linear-gradient(135deg, rgba(255, 245, 244, 0.96), rgba(252, 232, 229, 0.94)),
    #fff1f0;
}

.activity-chip--error .activity-chip__icon {
  color: #b24b43;
  background: rgba(239, 197, 194, 0.78);
}

.activity-chip--cancelled {
  border-color: #cbd5e1;
  background:
    linear-gradient(135deg, rgba(248, 250, 252, 0.98), rgba(241, 245, 249, 0.94)),
    #f8fafc;
}

.activity-chip--cancelled .activity-chip__icon {
  color: #64748b;
  background: rgba(226, 232, 240, 0.82);
}

.activity-chip--waiting {
  border-color: #bfdbfe;
  background:
    linear-gradient(135deg, rgba(239, 246, 255, 0.98), rgba(219, 234, 254, 0.9)),
    #eff6ff;
}

.activity-chip--waiting .activity-chip__icon {
  color: #2563eb;
  background: rgba(191, 219, 254, 0.82);
}

.activity-spinner {
  width: 11px;
  height: 11px;
  border-radius: 999px;
  border: 2px solid rgba(37, 99, 235, 0.22);
  border-top-color: currentColor;
  animation: activity-spin 0.9s linear infinite;
}

.activity-pulse {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  background: currentColor;
  animation: activity-pulse 1.2s ease-in-out infinite;
}

.activity-chip-enter-active,
.activity-chip-leave-active {
  transition: all 0.22s ease;
}

.activity-chip-enter-from,
.activity-chip-leave-to {
  opacity: 0;
  transform: translateY(6px) scale(0.98);
}

.dark .activity-chip {
  border-color: #334155;
  background:
    radial-gradient(circle at top left, rgba(51, 65, 85, 0.92), rgba(31, 41, 55, 0.96)),
    #1f2937;
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.22);
}

.dark .activity-chip__title {
  color: #f8fafc;
}

.dark .activity-chip__detail {
  color: #cbd5e1;
}

.dark .activity-chip--running {
  border-color: #1e40af;
  background:
    linear-gradient(135deg, rgba(30, 64, 175, 0.35), rgba(30, 58, 138, 0.34)),
    #172554;
}

.dark .activity-chip--running .activity-chip__icon {
  color: #bfdbfe;
  background: rgba(59, 130, 246, 0.34);
}

.dark .activity-chip--completed {
  border-color: #395b49;
  background:
    linear-gradient(135deg, rgba(29, 54, 42, 0.95), rgba(24, 44, 34, 0.96)),
    #1d362a;
}

.dark .activity-chip--completed .activity-chip__icon {
  color: #9dd8b6;
  background: rgba(67, 108, 84, 0.5);
}

.dark .activity-chip--error {
  border-color: #6f3b37;
  background:
    linear-gradient(135deg, rgba(75, 32, 31, 0.95), rgba(63, 27, 27, 0.96)),
    #47211f;
}

.dark .activity-chip--error .activity-chip__icon {
  color: #f2a5a0;
  background: rgba(117, 56, 52, 0.5);
}

.dark .activity-chip--cancelled {
  border-color: #475569;
  background:
    linear-gradient(135deg, rgba(51, 65, 85, 0.92), rgba(30, 41, 59, 0.96)),
    #1e293b;
}

.dark .activity-chip--cancelled .activity-chip__icon {
  color: #cbd5e1;
  background: rgba(100, 116, 139, 0.36);
}

.dark .activity-chip--waiting {
  border-color: #1e40af;
  background:
    linear-gradient(135deg, rgba(30, 64, 175, 0.35), rgba(30, 58, 138, 0.34)),
    #172554;
}

.dark .activity-chip--waiting .activity-chip__icon {
  color: #bfdbfe;
  background: rgba(59, 130, 246, 0.34);
}

@keyframes activity-spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes activity-pulse {
  0%,
  100% {
    transform: scale(0.86);
    opacity: 0.42;
  }

  50% {
    transform: scale(1);
    opacity: 0.95;
  }
}
</style>
