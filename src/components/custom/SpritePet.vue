```vue
<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'

const props = defineProps<{
  spritesheetUrl: string
  cellSize: string
  atlasSize: string
  fps?: number
}>()

const frame = ref(0)
let timer: number | null = null

const cell = computed(() => {
  if (!props.cellSize) {
    return { width: 192, height: 208 }
  }
  const [w, h] = props.cellSize.split('x').map(Number)

  return {
    width: Number.isFinite(w) ? w : 192,
    height: Number.isFinite(h) ? h : 208,
  }
})

const atlas = computed(() => {
  if (!props.atlasSize) {
    return { width: 1536, height: 1872 }
  }
  const [w, h] = props.atlasSize.split('x').map(Number)

  return {
    width: Number.isFinite(w) ? w : 1536,
    height: Number.isFinite(h) ? h : 1872,
  }
})


const totalFrames = computed(() => {
  const cols = Math.floor(atlas.value.width / cell.value.width)
  const rows = Math.floor(atlas.value.height / cell.value.height)
  return Math.max(cols * rows, 1)
})

const backgroundPosition = computed(() => {
  return `${-(frame.value * cell.value.width)}px 0px`
})

const style = computed(() => ({
  width: `${cell.value.width}px`,
  height: `${cell.value.height}px`,
  backgroundImage: `url(${props.spritesheetUrl})`,
  backgroundRepeat: 'no-repeat',
  backgroundPosition: backgroundPosition.value,
  backgroundSize: `${atlas.value.width}px ${atlas.value.height}px`,
}))

function start() {
  if (timer !== null) return

  const fps = props.fps ?? 5
  const interval = Math.floor(1000 / fps)

  timer = window.setInterval(() => {
    frame.value++

    if (frame.value >= totalFrames) {
      frame.value = 0
    }
  }, interval)
}

function stop() {
  if (timer !== null) {
    clearInterval(timer)
    timer = null
  }

  frame.value = 0
}

watch(
  () => props.spritesheetUrl,
  () => {
    stop()
  }
)

onUnmounted(() => {
  stop()
})
</script>

<template>
  <div
    class="select-none"
    :style="style"
    @mouseenter="start"
    @mouseleave="stop"
  />
</template>
```
