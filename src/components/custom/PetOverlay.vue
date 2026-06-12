<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

interface PetConfig {
  cell_width: number
  cell_height: number
  atlas_width: number
  atlas_height: number
  row_frame_counts: number[]
  spritesheet_url: string
}

const config = ref<PetConfig | null>(null)
const currentRow = ref(0)
const currentFrame = ref(0)
let timer = 0
let stateTimer = 0

function render() {
  if (!config.value) return
  const el = document.getElementById('pet-sprite') as HTMLElement
  if (!el) return
  const cw = config.value.cell_width
  const ch = config.value.cell_height
  const x = -(currentFrame.value * cw)
  const y = -(currentRow.value * ch)
  el.style.width = cw + 'px'
  el.style.height = ch + 'px'
  el.style.backgroundImage = `url(${config.value.spritesheet_url})`
  el.style.backgroundSize = `${config.value.atlas_width}px ${config.value.atlas_height}px`
  el.style.backgroundPosition = `${x}px ${y}px`
  el.style.backgroundRepeat = 'no-repeat'
}

function startAnimation() {
  stopAnimation()
  if (!config.value) return
  const maxFrames = config.value.row_frame_counts[currentRow.value] || 6
  timer = window.setInterval(() => {
    currentFrame.value++
    if (currentFrame.value >= maxFrames) currentFrame.value = 0
    render()
  }, 300)
}

function stopAnimation() {
  if (timer) { clearInterval(timer); timer = 0 }
  currentFrame.value = 0
}

function startRandomStates() {
  stopRandomStates()
  if (!config.value || config.value.row_frame_counts.length <= 1) return

  function scheduleNext() {
    const delay = 5000 + Math.random() * 15000
    stateTimer = window.setTimeout(() => {
      if (!config.value) return
      const total = config.value.row_frame_counts.length
      const r = Math.random()

      let next: number
      if (r < 0.5) {
        next = 0
      } else if (r < 0.7) {
        next = 3
      } else if (r < 0.85) {
        next = 4
      } else {
        const others = [1, 2, 5, 6, 7, 8].filter(i => i < total)
        next = others[Math.floor(Math.random() * others.length)] ?? 0
      }

      switchState(next)

      const duration = (config.value!.row_frame_counts[next] || 6) * 200 + 500
      stateTimer = window.setTimeout(() => {
        if (next !== 0) {
          switchState(0)
        }
        scheduleNext()
      }, duration)
    }, delay)
  }

  scheduleNext()
}

const showContextMenu = ref<{ x: number; y: number } | null>(null)

function stopRandomStates() {
  if (stateTimer) { clearTimeout(stateTimer); stateTimer = 0 }
}

async function closePet() {
  try {
    await invoke('close_desktop_pet')
  } catch (e) { console.warn(e) }
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault()
  showContextMenu.value = { x: e.clientX, y: e.clientY }
}

function onMouseDown(e: MouseEvent) {
  const target = e.target as HTMLElement
  if (!target.closest('.pet-context-menu')) {
    showContextMenu.value = null
  }
}

function switchState(row: number) {
  currentRow.value = row
  stopAnimation()
  startAnimation()
}

async function loadConfig() {
  try {
    const urlParams = new URLSearchParams(window.location.search)
    const petId = urlParams.get('petId')
    if (petId) {
      const data = await invoke<PetConfig>('get_pet_window_config')
      if (data) {
        config.value = data
      }
    }
  } catch (e) {
    console.warn('Failed to load pet config:', e)
  }
}

let unlisten: (() => void) | null = null

onMounted(async () => {
  await loadConfig()

  unlisten = await listen<PetConfig>('pet-window-update', (event) => {
    config.value = event.payload
    currentRow.value = 0
    stopAnimation()
    stopRandomStates()
    render()
    startAnimation()
    startRandomStates()
  })

  render()
  startAnimation()
  startRandomStates()

  const container = document.getElementById('pet-container')
  if (container) {
    container.addEventListener('mousedown', async (e: MouseEvent) => {
      if ((e.target as HTMLElement).classList.contains('close-btn')) return
      try {
        await getCurrentWindow().startDragging()
      } catch (err) { console.warn(err) }
    })
  }
})

onUnmounted(() => {
  stopAnimation()
  stopRandomStates()
  unlisten?.()
})
</script>

<template>
  <div
    id="pet-container"
    class="flex h-screen w-screen items-center justify-center bg-transparent select-none"
    @contextmenu.prevent="onContextMenu"
    @mousedown="onMouseDown"
  >
    <div id="pet-sprite" class="bg-no-repeat" />
    <div
      v-if="showContextMenu"
      class="pet-context-menu fixed z-50 rounded-lg border border-gray-200 bg-white py-1 shadow-lg"
      :style="{ left: showContextMenu.x + 'px', top: showContextMenu.y + 'px' }"
      @click.stop
    >
      <button
        class="close-btn w-full px-4 py-1.5 text-left text-sm text-red-600 hover:bg-red-50"
        @click="closePet"
      >
        关闭宠物
      </button>
    </div>
  </div>
</template>
