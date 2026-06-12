<script setup lang="ts">
import { ref, computed } from 'vue'
import SpritePet from './SpritePet.vue'

const props = defineProps<{
  petId: string
  displayName: string
  spritesheetUrl: string
  cellSize: string
  atlasSize: string
  rowFrameCounts: number[]
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'detach'): void
}>()

const STATES = [
  { label: 'Idle', icon: '💤' },
  { label: 'Run right', icon: '➡️' },
  { label: 'Run left', icon: '⬅️' },
  { label: 'Waving', icon: '👋' },
  { label: 'Jumping', icon: '⬆️' },
  { label: 'Failed', icon: '❌' },
  { label: 'Waiting', icon: '⏳' },
  { label: 'Running', icon: '🏃' },
  { label: 'Review', icon: '🔍' },
]

const activeRow = ref(0)

const visibleStates = computed(() => {
  return STATES.filter((_, i) => i < props.rowFrameCounts.length)
})
</script>

<template>
  <div class="pet-popup">
    <div class="pet-popup-header">
      <span class="pet-popup-title">{{ displayName }}</span>
      <div class="pet-popup-actions">
        <button class="pet-popup-btn" title="置顶到桌面" @click="emit('detach')">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="12" y1="3" x2="12" y2="21"/><line x1="3" y1="12" x2="21" y2="12"/></svg>
        </button>
        <button class="pet-popup-btn" title="关闭" @click="emit('close')">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
    </div>

    <div class="pet-popup-sprite">
      <SpritePet
        :spritesheet-url="spritesheetUrl"
        :cell-size="cellSize"
        :atlas-size="atlasSize"
        :row="activeRow"
        :row-frame-counts="rowFrameCounts"
        :fps="5"
      />
    </div>

    <div class="pet-popup-states">
      <button
        v-for="(state, index) in visibleStates"
        :key="state.label"
        class="pet-state-btn"
        :class="{ active: activeRow === index }"
        @click="activeRow = index"
      >
        <span class="pet-state-icon">{{ state.icon }}</span>
        <span class="pet-state-label">{{ state.label }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.pet-popup {
  position: fixed;
  bottom: 20px;
  right: 20px;
  width: 280px;
  background: white;
  border-radius: 16px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
  border: 1px solid #e7e2d8;
  z-index: 1000;
  overflow: hidden;
  font-family: system-ui, -apple-system, sans-serif;
}

.pet-popup-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  background: #faf9f6;
  border-bottom: 1px solid #e7e2d8;
}

.pet-popup-title {
  font-size: 13px;
  font-weight: 600;
  color: #111827;
}

.pet-popup-actions {
  display: flex;
  gap: 4px;
}

.pet-popup-btn {
  width: 26px;
  height: 26px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #6b7280;
  transition: all 0.15s;
}

.pet-popup-btn:hover {
  background: #e7e2d8;
  color: #111827;
}

.pet-popup-sprite {
  display: flex;
  justify-content: center;
  align-items: center;
  padding: 20px;
  background: #f3f1ed;
  min-height: 240px;
}

.pet-popup-states {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
  padding: 8px;
}

.pet-state-btn {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 4px;
  border: 1px solid transparent;
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.15s;
}

.pet-state-btn:hover {
  background: #f3f1ed;
}

.pet-state-btn.active {
  background: #111827;
  color: white;
}

.pet-state-icon {
  font-size: 16px;
}

.pet-state-label {
  font-size: 10px;
  color: #6b7280;
  white-space: nowrap;
}

.pet-state-btn.active .pet-state-label {
  color: white;
}
</style>
