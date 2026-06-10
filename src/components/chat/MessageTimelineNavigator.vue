<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';

export interface MessageTimelineItem {
  index: number;
  summary: string;
}

const props = defineProps<{
  items: MessageTimelineItem[];
  activeIndex?: number | null;
}>();

const emit = defineEmits<{
  (e: 'select', index: number): void;
}>();

const panelRef = ref<HTMLElement | null>(null);
const previewIndex = ref<number | null>(null);

const compactMode = computed(() => props.items.length > 28);

const railHeight = computed(() => {
  if (compactMode.value) return '300px';
  const height = Math.min(Math.max(props.items.length * 9, 32), 252);
  return `${height}px`;
});

const activePanelIndex = computed(() => previewIndex.value ?? props.activeIndex ?? null);

const focusPreview = async (index: number) => {
  previewIndex.value = index;
  await nextTick();
  panelRef.value
    ?.querySelector<HTMLElement>(`[data-timeline-index="${index}"]`)
    ?.scrollIntoView({ block: 'nearest' });
};

const clearPreview = () => {
  previewIndex.value = null;
};
</script>

<template>
  <nav
    v-if="items.length > 1"
    class="message-timeline-navigator"
    aria-label="用户消息导航"
  >
    <div
      class="message-timeline-lines"
      :class="{ 'is-scrollable': compactMode }"
      :style="{ height: railHeight }"
    >
      <button
        v-for="(item, order) in items"
        :key="item.index"
        type="button"
        class="message-timeline-line"
        :class="{ 'is-active': item.index === activeIndex }"
        :aria-label="`跳转到第 ${order + 1} 条用户消息`"
        :title="item.summary"
        @mouseenter="focusPreview(item.index)"
        @focus="focusPreview(item.index)"
        @click="emit('select', item.index)"
      ></button>
    </div>

    <div
      ref="panelRef"
      class="message-timeline-panel"
      role="list"
      aria-label="用户消息列表"
      @mouseleave="clearPreview"
    >
      <button
        v-for="(item, order) in items"
        :key="`panel-${item.index}`"
        type="button"
        class="message-timeline-item"
        :class="{ 'is-active': item.index === activePanelIndex }"
        role="listitem"
        :data-timeline-index="item.index"
        :title="item.summary"
        @mouseenter="previewIndex = item.index"
        @focus="previewIndex = item.index"
        @click="emit('select', item.index)"
      >
        <span class="message-timeline-index">{{ order + 1 }}</span>
        <span class="message-timeline-summary">{{ item.summary }}</span>
      </button>
    </div>
  </nav>
</template>

<style scoped>
.message-timeline-navigator {
  position: absolute;
  top: 50%;
  right: -54px;
  z-index: 9;
  transform: translateY(-50%);
}

.message-timeline-lines {
  width: 36px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  padding: 4px 0;
}

.message-timeline-lines.is-scrollable {
  justify-content: flex-start;
  gap: 6px;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding-right: 2px;
}

.message-timeline-lines.is-scrollable::-webkit-scrollbar {
  width: 3px;
}

.message-timeline-lines.is-scrollable::-webkit-scrollbar-track {
  background: transparent;
}

.message-timeline-lines.is-scrollable::-webkit-scrollbar-thumb {
  border-radius: 999px;
  background: #d4d4d4;
}

.message-timeline-line {
  width: 26px;
  height: 3px;
  flex: 0 0 auto;
  border: 0;
  border-radius: 999px;
  background: #b9b9b9;
  cursor: pointer;
  transition: width 0.16s ease, background 0.16s ease, opacity 0.16s ease;
}

.message-timeline-line:hover,
.message-timeline-line:focus-visible,
.message-timeline-line.is-active {
  width: 28px;
  background: #111827;
}

.message-timeline-line:focus-visible {
  outline: 2px solid rgba(37, 99, 235, 0.3);
  outline-offset: 3px;
}

.message-timeline-panel {
  position: absolute;
  top: 50%;
  right: 34px;
  width: min(420px, calc(100vw - 116px));
  max-height: min(68vh, 560px);
  overflow-y: auto;
  padding: 10px;
  border: 1px solid rgba(203, 213, 225, 0.92);
  border-radius: 18px;
  background: rgba(255, 255, 255, 0.98);
  box-shadow: 0 22px 54px rgba(15, 23, 42, 0.15), 0 4px 12px rgba(15, 23, 42, 0.08);
  transform: translateY(-50%) translateX(8px);
  opacity: 0;
  pointer-events: none;
  backdrop-filter: blur(16px);
  transition: opacity 0.16s ease, transform 0.16s ease;
}

.message-timeline-navigator:hover .message-timeline-panel,
.message-timeline-navigator:focus-within .message-timeline-panel {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(-50%) translateX(0);
}

.message-timeline-panel::-webkit-scrollbar {
  width: 5px;
}

.message-timeline-panel::-webkit-scrollbar-track {
  background: transparent;
}

.message-timeline-panel::-webkit-scrollbar-thumb {
  border-radius: 999px;
  background: #c4c4c4;
}

.message-timeline-item {
  width: 100%;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: #111827;
  cursor: pointer;
  padding: 8px 10px;
  text-align: left;
  transition: background 0.14s ease, color 0.14s ease;
}

.message-timeline-item:hover,
.message-timeline-item:focus-visible,
.message-timeline-item.is-active {
  background: #f3f4f6;
}

.message-timeline-item:focus-visible {
  outline: 2px solid rgba(37, 99, 235, 0.24);
  outline-offset: 2px;
}

.message-timeline-index {
  width: 22px;
  flex: 0 0 auto;
  color: #94a3b8;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.message-timeline-summary {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  color: inherit;
  font-size: 14px;
  line-height: 1.45;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dark .message-timeline-line {
  background: #6b7280;
}

.dark .message-timeline-line:hover,
.dark .message-timeline-line:focus-visible,
.dark .message-timeline-line.is-active {
  background: #f8fafc;
}

.dark .message-timeline-lines.is-scrollable::-webkit-scrollbar-thumb {
  background: #4b5563;
}

.dark .message-timeline-panel {
  border-color: rgba(71, 85, 105, 0.94);
  background: rgba(31, 41, 55, 0.98);
  box-shadow: 0 22px 54px rgba(0, 0, 0, 0.36), 0 4px 12px rgba(0, 0, 0, 0.22);
}

.dark .message-timeline-panel::-webkit-scrollbar-thumb {
  background: #64748b;
}

.dark .message-timeline-item {
  color: #f8fafc;
}

.dark .message-timeline-item:hover,
.dark .message-timeline-item:focus-visible,
.dark .message-timeline-item.is-active {
  background: rgba(255, 255, 255, 0.08);
}

.dark .message-timeline-index {
  color: #94a3b8;
}

@media (max-width: 1040px) {
  .message-timeline-navigator {
    display: none;
  }
}
</style>
