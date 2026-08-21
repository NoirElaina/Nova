<script setup lang="ts">
// 选中文本后的浮动操作条：引用到主对话 / 开分支提问。
// 通过 Teleport 挂到 body，fixed 定位在选区上方居中。
// 按钮用 mousedown.prevent 阻止点击时选区塌陷，click 才触发动作。
// 视觉对齐主 app 的白底黑字语言。
defineProps<{
  visible: boolean;
  x: number;
  y: number;
}>();

const emit = defineEmits<{
  (e: 'quote'): void;
  (e: 'branch'): void;
}>();
</script>

<template>
  <Teleport to="body">
    <div
      v-if="visible"
      data-selection-popover
      class="selection-popover"
      :style="{ left: `${x}px`, top: `${y}px` }"
      role="toolbar"
      aria-label="选中文本操作"
    >
      <button
        type="button"
        class="selection-popover-btn"
        title="把选中文本作为引用插入主对话输入框"
        @mousedown.prevent
        @click="emit('quote')"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M3 21c3 0 7-1 7-8V5c0-1.25-.757-2.017-2-2H4c-1.25 0-2 .75-2 1.972V11c0 1.25.75 2 2 2 1 0 1 0 1 1v1c0 1-1 2-2 2s-1 .008-1 1.031V20c0 1 0 1 1 1z" />
          <path d="M15 21c3 0 7-1 7-8V5c0-1.25-.757-2.017-2-2h-4c-1.25 0-2 .75-2 1.972V11c0 1.25.75 2 2 2h.75c0 2.25.25 4-2.75 4v3c0 1 0 1 1 1z" />
        </svg>
        引用到对话
      </button>
      <span class="selection-popover-divider" aria-hidden="true"></span>
      <button
        type="button"
        class="selection-popover-btn selection-popover-btn-primary"
        title="针对选中文本开启临时分支对话，不影响主对话上下文"
        @mousedown.prevent
        @click="emit('branch')"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="6" y1="3" x2="6" y2="15" />
          <circle cx="18" cy="6" r="3" />
          <circle cx="6" cy="18" r="3" />
          <path d="M18 9a9 9 0 0 1-9 9" />
        </svg>
        分支提问
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
.selection-popover {
  position: fixed;
  transform: translate(-50%, calc(-100% - 8px));
  z-index: 70;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px;
  background: rgba(255, 255, 255, 0.98);
  border: 1px solid #e5e5e5;
  border-radius: 10px;
  box-shadow: 0 8px 24px rgba(15, 23, 42, 0.12), 0 2px 6px rgba(15, 23, 42, 0.06);
  animation: selection-popover-in 0.12s ease-out;
}

@keyframes selection-popover-in {
  from {
    opacity: 0;
    transform: translate(-50%, calc(-100% - 4px));
  }
  to {
    opacity: 1;
    transform: translate(-50%, calc(-100% - 8px));
  }
}

.selection-popover-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 9px;
  font-size: 12px;
  line-height: 1;
  color: #334155;
  background: transparent;
  border: none;
  border-radius: 7px;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.12s ease, color 0.12s ease;
}

.selection-popover-btn:hover {
  background: #f1f5f9;
  color: #0f172a;
}

.selection-popover-btn-primary {
  font-weight: 600;
  color: #0f172a;
}

.selection-popover-divider {
  width: 1px;
  height: 14px;
  background: #e5e5e5;
}

.dark .selection-popover {
  background: rgba(38, 38, 38, 0.98);
  border-color: #2e2e2e;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4), 0 2px 6px rgba(0, 0, 0, 0.2);
}

.dark .selection-popover-btn {
  color: #cbd5e1;
}

.dark .selection-popover-btn:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #f1f5f9;
}

.dark .selection-popover-btn-primary {
  color: #f1f5f9;
}

.dark .selection-popover-divider {
  background: #2e2e2e;
}
</style>
