<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import SpritePet from './SpritePet.vue'

interface ValidationReport {
  manifestId: string
  atlasSize: string
  cellSize: string
  statesDetected: number
  manifestBytes: number
  spritesheetBytes: number
}

interface Pet {
  id: string
  displayName: string
  description: string
  kind: string
  ownerHandle: string
  ownerName: string
  tags: string[]
  viewCount: number
  downloadCount: number
  likeCount: number
  commentCount: number
  previewUrl: string
  posterUrl: string
  spritesheetUrl: string
  shareImageUrl: string
  downloadUrl: string
  validationReport: ValidationReport
}

interface PetsResponse {
  pets: Pet[]
  page: number
  pageSize: number
  total: number
  totalPages: number
}

interface FetchPetRequest {
  page: number
  pageSize: number
  sort: string
  kind?: string
  tag?: string
}

const loading = ref(false)
const error = ref('')

const pets = ref<Pet[]>([])

const total = ref(0)
const totalPages = ref(1)

const keyword = ref('')

const query = ref<FetchPetRequest>({
  page: 1,
  pageSize: 30,
  sort: 'popular',
  kind: 'all',
  tag: 'all',
})

const sortOptions = [
  { label: 'Liked', value: 'popular' },
  { label: 'Discussed', value: 'discussed' },
  { label: 'Random', value: 'random' },
]

const kindOptions = [
  { label: 'ALL', value: 'all' },
  { label: 'Object', value: 'object' },
  { label: 'Animal', value: 'animal' },
  { label: 'Person', value: 'person' },
  { label: 'Creature', value: 'creature' },
]

const tagOptions = [
  'all',
  'cute',
  'anime',
  'pixel',
  'game',
]

async function loadPets() {
  loading.value = true
  error.value = ''

  try {
    const data = await invoke<PetsResponse>('fetch_pet', {
      page: query.value.page,
      pageSize: query.value.pageSize,
      sort: query.value.sort,
      kind: query.value.kind,
      tag: query.value.tag,
    })

    pets.value = data.pets
    total.value = data.total
    totalPages.value = data.totalPages
  }
  catch (e) {
    console.error(e)
    error.value = String(e)
  }
  finally {
    loading.value = false
  }
}

const filteredPets = computed(() => {
  if (!keyword.value.trim()) {
    return pets.value
  }

  const q = keyword.value.toLowerCase()

  return pets.value.filter(
    pet =>
      pet.displayName.toLowerCase().includes(q)
      || pet.description.toLowerCase().includes(q)
      || pet.ownerName.toLowerCase().includes(q),
  )
})

function changeSort(sort: string) {
  query.value.sort = sort
  query.value.page = 1
  loadPets()
}

function changeKind(kind: string) {
  query.value.kind = kind
  query.value.page = 1
  loadPets()
}

function changeTag(tag: string) {
  query.value.tag = tag
  query.value.page = 1
  loadPets()
}

function prevPage() {
  if (query.value.page <= 1) {
    return
  }

  query.value.page--
  loadPets()
}

function nextPage() {
  if (query.value.page >= totalPages.value) {
    return
  }

  query.value.page++
  loadPets()
}

const downloadingId = ref<string | null>(null)

async function downloadPet(pet: Pet) {
  if (downloadingId.value) return
  downloadingId.value = pet.id
  try {
    await invoke<string>('download_pet', {
      petId: pet.id,
      displayName: pet.displayName,
      downloadUrl: pet.downloadUrl,
      cellSize: pet.validationReport?.cellSize ?? '192x208',
      atlasSize: pet.validationReport?.atlasSize ?? '1536x1872',
      rowFrameCounts: [6, 8, 8, 4, 5, 8, 6, 6, 6],
    })
    await loadLocalPets()
  } catch (e) {
    console.error('Download failed:', e)
  } finally {
    downloadingId.value = null
  }
}

interface LocalPet {
  id: string
  display_name: string
  cell_size: string
  atlas_size: string
  row_frame_counts: number[]
}

const localPets = ref<LocalPet[]>([])
const localSpritesheets = ref<Record<string, string>>({})
const showMyPets = ref(false)

async function launchPet(pet: LocalPet) {
  try {
    await invoke('launch_desktop_pet', {
      petId: pet.id,
      cellSize: pet.cell_size,
      atlasSize: pet.atlas_size,
      rowFrameCounts: pet.row_frame_counts,
    })
    showMyPets.value = false
  } catch (e) {
    console.error('Failed to launch pet:', e)
  }
}

async function loadLocalPets() {
  try {
    localPets.value = await invoke<LocalPet[]>('list_local_pets')
    for (const pet of localPets.value) {
      if (!localSpritesheets.value[pet.id]) {
        const dataUrl = await invoke<string>('get_pet_spritesheet', { petId: pet.id })
        localSpritesheets.value[pet.id] = dataUrl
      }
    }
  } catch (e) {
    console.error('Failed to load local pets:', e)
  }
}

onMounted(() => {
  loadPets()
  loadLocalPets()
})
</script>

<template>
  <div class="h-full overflow-y-auto bg-[#faf9f6] px-6 pt-16 pb-6">

    <div class="mb-3 flex items-center gap-3">
      <h1 class="text-xl font-bold text-[#111827]">
        Codex Pets
      </h1>

      <p class="text-xs text-[#6b7280]">
        Discover and collect animated pets
      </p>

      <div class="ml-auto">
        <button
          class="inline-flex items-center gap-1.5 rounded-lg border border-[#e7e2d8] bg-white px-3 py-1.5 text-xs text-[#111827] transition hover:bg-[#f3f1ed]"
          @click="showMyPets = !showMyPets"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/></svg>
          我的宠物
          <span v-if="localPets.length" class="rounded-full bg-black px-1.5 py-0.5 text-[10px] text-white">{{ localPets.length }}</span>
        </button>
      </div>
    </div>

    <div class="mb-3 flex gap-2">
      <input
        v-model="keyword"
        placeholder="Search pets..."
        class="flex-1 rounded-lg border border-[#e7e2d8] bg-white px-3 py-1.5 text-sm outline-none"
      >

      <button
        class="rounded-lg bg-black px-4 py-1.5 text-sm text-white"
      >
        Find
      </button>
    </div>

    <div class="mb-3 flex flex-wrap gap-2">

      <div
        class="flex overflow-hidden rounded-full border border-[#e7e2d8] bg-white"
      >
        <button
          v-for="item in sortOptions"
          :key="item.value"
          class="px-3 py-1 text-xs"
          :class="
            query.sort === item.value
              ? 'bg-black text-white'
              : 'text-[#6b7280]'
          "
          @click="changeSort(item.value)"
        >
          {{ item.label }}
        </button>
      </div>

      <div
        class="flex overflow-hidden rounded-full border border-[#e7e2d8] bg-white"
      >
        <button
          v-for="item in kindOptions"
          :key="item.value"
          class="px-3 py-1 text-xs"
          :class="
            query.kind === item.value
              ? 'bg-black text-white'
              : 'text-[#6b7280]'
          "
          @click="changeKind(item.value)"
        >
          {{ item.label }}
        </button>
      </div>

      <select
        class="rounded-lg border border-[#e7e2d8] bg-white px-3 py-1 text-xs"
        :value="query.tag"
        @change="changeTag(($event.target as HTMLSelectElement).value)"
      >
        <option
          v-for="tag in tagOptions"
          :key="tag"
          :value="tag"
        >
          {{ tag }}
        </option>
      </select>
    </div>

    <div
      class="mb-4 flex items-center justify-between text-xs text-[#6b7280]"
    >
      <div>
        {{ total.toLocaleString() }} pets
      </div>

      <div>
        Page {{ query.page }} / {{ totalPages }}
      </div>
    </div>

    <div
      v-if="loading"
      class="py-20 text-center text-[#6b7280]"
    >
      Loading...
    </div>

    <div
      v-else-if="error"
      class="py-20 text-center text-red-500"
    >
      {{ error }}
    </div>

    <div
      v-else
      class="grid grid-cols-1 gap-5 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4"
    >
      <article
        v-for="pet in filteredPets"
        :key="pet.id"
        class="group overflow-hidden rounded-2xl border border-[#e7e2d8] bg-white shadow-sm transition hover:-translate-y-1 hover:shadow-lg"
      >
        <div class="bg-[#fcfbf8] p-4">
          <div class="flex justify-center">
            <SpritePet
              :spritesheet-url="pet.spritesheetUrl"
              :cell-size="pet.validationReport?.cellSize ?? ''"
              :atlas-size="pet.validationReport?.atlasSize ?? ''"
              :frame-count="6"
              :fps="5"
            />
          </div>
        </div>

        <div class="p-4">
          <h3 class="text-lg font-semibold">
            {{ pet.displayName }}
          </h3>

          <div class="mb-2 text-sm text-[#6b7280]">
            by {{ pet.ownerName }}
          </div>

          <p class="line-clamp-3 text-sm text-[#6b7280]">
            {{ pet.description }}
          </p>

          <div class="mt-3 flex flex-wrap gap-2">
            <span
              v-for="tag in pet.tags.slice(0, 6)"
              :key="tag"
              class="rounded-full border px-2 py-1 text-xs"
            >
              {{ tag }}
            </span>
          </div>

          <div class="mt-4 flex items-center justify-between text-sm text-[#6b7280]">
            <div class="flex gap-4">
              <span>👁 {{ pet.viewCount }}</span>
              <span>♡ {{ pet.likeCount }}</span>
              <span>⬇ {{ pet.downloadCount }}</span>
            </div>
            <button
              :disabled="downloadingId === pet.id"
              class="inline-flex items-center gap-1 rounded-lg bg-black px-3 py-1.5 text-xs text-white transition hover:bg-gray-800 disabled:opacity-50"
              @click="downloadPet(pet)"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
                <polyline points="7 10 12 15 17 10"/>
                <line x1="12" y1="15" x2="12" y2="3"/>
              </svg>
              {{ downloadingId === pet.id ? 'Saving...' : 'Download' }}
            </button>
          </div>
        </div>
      </article>
    </div>

    <div
      class="mt-8 flex items-center justify-center gap-4"
    >
      <button
        class="rounded-xl border bg-white px-4 py-2 disabled:opacity-40"
        :disabled="query.page <= 1"
        @click="prevPage"
      >
        Previous
      </button>

      <div class="text-sm text-[#6b7280]">
        {{ query.page }} / {{ totalPages }}
      </div>

      <button
        class="rounded-xl border bg-white px-4 py-2 disabled:opacity-40"
        :disabled="query.page >= totalPages"
        @click="nextPage"
      >
        Next
      </button>
    </div>

    <div v-if="showMyPets && localPets.length > 0" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30" @click.self="showMyPets = false">
      <div class="w-[480px] max-w-[90vw] rounded-2xl border border-[#e7e2d8] bg-white p-5 shadow-2xl">
        <div class="mb-4 flex items-center justify-between">
          <h3 class="text-sm font-semibold text-[#111827]">选择要放置到桌面的宠物</h3>
          <button class="text-xs text-[#6b7280] hover:text-[#111827]" @click="showMyPets = false">关闭</button>
        </div>
        <div class="grid grid-cols-3 gap-3">
          <button
            v-for="pet in localPets"
            :key="pet.id"
            class="group flex flex-col items-center gap-2 rounded-xl border border-[#e7e2d8] p-3 transition hover:-translate-y-1 hover:border-black hover:shadow-lg"
            @click="launchPet(pet)"
          >
            <div class="flex justify-center">
              <SpritePet
                :spritesheet-url="localSpritesheets[pet.id] ?? ''"
                :cell-size="pet.cell_size"
                :atlas-size="pet.atlas_size"
                :row-frame-counts="pet.row_frame_counts"
                :fps="5"
              />
            </div>
            <span class="text-xs text-[#6b7280]">{{ pet.display_name }}</span>
          </button>
        </div>
      </div>
    </div>

  </div>
</template>