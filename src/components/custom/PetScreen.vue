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
  { label: 'Newest', value: 'newest' },
  { label: 'Viewed', value: 'viewed' },
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

onMounted(loadPets)
</script>

<template>
  <div class="h-full overflow-y-auto bg-[#faf9f6] px-6 py-6">

    <div class="mb-6">
      <h1 class="text-3xl font-bold text-[#111827]">
        Codex Pets
      </h1>

      <p class="mt-2 text-sm text-[#6b7280]">
        Discover and collect animated pets
      </p>
    </div>

    <div class="mb-6 flex gap-3">
      <input
        v-model="keyword"
        placeholder="Search pets..."
        class="flex-1 rounded-xl border border-[#e7e2d8] bg-white px-4 py-3 outline-none"
      >

      <button
        class="rounded-xl bg-black px-6 py-3 text-white"
      >
        Find
      </button>
    </div>

    <div class="mb-4 flex flex-wrap gap-3">

      <div
        class="flex overflow-hidden rounded-full border border-[#e7e2d8] bg-white"
      >
        <button
          v-for="item in sortOptions"
          :key="item.value"
          class="px-4 py-2 text-sm"
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
          class="px-4 py-2 text-sm"
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
        class="rounded-xl border border-[#e7e2d8] bg-white px-4"
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
      class="mb-6 flex items-center justify-between text-sm text-[#6b7280]"
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
              :cell-size="pet.validationReport.cellSize"
              :atlas-size="pet.validationReport.atlasSize"
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

          <div class="mt-4 flex gap-4 text-sm text-[#6b7280]">
            <span>👁 {{ pet.viewCount }}</span>
            <span>♡ {{ pet.likeCount }}</span>
            <span>⬇ {{ pet.downloadCount }}</span>
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

  </div>
</template>