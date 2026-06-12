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
let timer: number | null = null
let stateTimer: number | null = null

const STATES = ['Idle','Run R','Run L','Wave','Jump','Fail','Wait','Run','Review']
const STATE_ICONS = ['💤','➡️','⬅️','👋','⬆️','❌','⏳','🏃','🔍']

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
  }, 200)
}

function stopAnimation() {
  if (timer) { clearInterval(timer); timer = null }
  currentFrame.value = 0
}

function startRandomStates() {
  stopRandomStates()
  if (!config.value || config.value.row_frame_counts.length <= 1) return
  stateTimer = window.setInterval(() => {
    const total = config.value!.row_frame_counts.length
    let next: number
    do {
      next = Math.floor(Math.random() * total)
    } while (next === currentRow.value && total > 1)
    switchState(next)
  }, 3000 + Math.random() * 4000)
}

function stopRandomStates() {
  if (stateTimer) { clearInterval(stateTimer); stateTimer = null }
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
      if ((e.target as HTMLElement).classList.contains('state-btn')) return
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
    class="flex h-screen w-screen cursor-grab items-center justify-center bg-transparent select-none active:cursor-grabbing"
  >
    <div id="pet-sprite" class="bg-no-repeat" />
    <div
      class="fixed bottom-1 left-1/2 flex gap-0.5 rounded-lg bg-black/65 p-0.5 opacity-0 transition-opacity hover:opacity-100"
      :class="{ 'opacity-100': false }"
      style="transform: translateX(-50%)"
      @mouseenter="($event.currentTarget as HTMLElement).style.opacity = '1'"
      @mouseleave="($event.currentTarget as HTMLElement).style.opacity = '0'"
    >
      <button
        v-for="(state, i) in STATES.slice(0, config?.row_frame_counts.length ?? 0)"
        :key="state"
        class="flex h-6 w-6 items-center justify-center rounded text-xs text-white transition-colors"
        :class="currentRow === i ? 'bg-white/50' : 'bg-white/12 hover:bg-white/30'"
        :title="state"
        @click.stop="switchState(i)"
      >
        {{ STATE_ICONS[i] }}
      </button>
    </div>
  </div>
</template>
