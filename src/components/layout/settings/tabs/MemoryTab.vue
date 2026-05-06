<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { emitToast } from '../../../../lib/toast'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import { Textarea } from '@/components/ui/textarea'
import {
  getStoredUiLanguage,
  normalizeUiLanguage,
  type UiLanguage,
} from '../../../../lib/ui-preferences'

type GlobalMemoryKind = 'preference' | 'fact' | 'rule'

type GlobalMemoryEntry = {
  id: string
  content: string
  kind: GlobalMemoryKind | string
  source: string
  hits: number
  createdAt: number
  updatedAt: number
}

const uiLanguage = ref<UiLanguage>(getStoredUiLanguage())
const isLoadingGlobalMemory = ref(false)
const isSavingGlobalMemory = ref(false)
const isClearingGlobalMemory = ref(false)
const isRemovingGlobalMemoryId = ref<string | null>(null)
const confirmDialogOpen = ref(false)
const globalMemoryEntries = ref<GlobalMemoryEntry[]>([])
const newGlobalMemoryContent = ref('')
const newGlobalMemoryKind = ref<GlobalMemoryKind>('fact')

const localeTexts = {
  'zh-CN': {
    title: '全局记忆',
    desc: '跨会话长期保留的偏好、事实和规则。AI 可自动写入；你也可以在这里手动管理。',
    inputPlaceholder: '例如：默认用中文回复；代码修改优先最小化变更。',
    kindPreference: '偏好',
    kindFact: '事实',
    kindRule: '规则',
    addButton: '新增记忆',
    adding: '新增中...',
    empty: '暂无全局记忆。',
    delete: '删除',
    clear: '清空全部记忆',
    clearing: '清空中...',
    confirmTitle: '清空全部全局记忆？',
    confirmDesc: '确认清空所有全局记忆吗？该操作不可撤销。',
    addDone: '已新增全局记忆。',
    addFailed: '新增全局记忆失败：',
    deleteDone: '已删除该条全局记忆。',
    deleteFailed: '删除全局记忆失败：',
    loadFailed: '加载全局记忆失败：',
    clearDone: '已清空 {count} 条全局记忆。',
    clearFailed: '清空全局记忆失败：',
    kindLabel: '类型',
    hits: '命中',
    cancel: '取消',
    confirm: '确认',
    loading: '加载中...',
  },
  'en-US': {
    title: 'Global Memory',
    desc: 'Persistent cross-session preferences, facts, and rules. AI can write them automatically, and you can manage them here.',
    inputPlaceholder: 'Example: Reply in Chinese by default; prefer minimal code changes.',
    kindPreference: 'Preference',
    kindFact: 'Fact',
    kindRule: 'Rule',
    addButton: 'Add Memory',
    adding: 'Adding...',
    empty: 'No global memory yet.',
    delete: 'Delete',
    clear: 'Clear All Memory',
    clearing: 'Clearing...',
    confirmTitle: 'Clear all global memory?',
    confirmDesc: 'Clear all global memory entries? This cannot be undone.',
    addDone: 'Global memory added.',
    addFailed: 'Failed to add global memory: ',
    deleteDone: 'Global memory entry deleted.',
    deleteFailed: 'Failed to delete global memory: ',
    loadFailed: 'Failed to load global memory: ',
    clearDone: 'Cleared {count} global memory entrie(s).',
    clearFailed: 'Failed to clear global memory: ',
    kindLabel: 'Type',
    hits: 'Hits',
    cancel: 'Cancel',
    confirm: 'Confirm',
    loading: 'Loading...',
  },
} as const

const t = computed(() => localeTexts[uiLanguage.value])
const globalMemoryKindOptions = computed(() => ([
  { value: 'preference' as GlobalMemoryKind, label: t.value.kindPreference },
  { value: 'fact' as GlobalMemoryKind, label: t.value.kindFact },
  { value: 'rule' as GlobalMemoryKind, label: t.value.kindRule },
]))
const formatClearDone = (count: number) => t.value.clearDone.replace('{count}', String(count))

const handleUiLanguageUpdated = (event: Event) => {
  const customEvent = event as CustomEvent<{ language?: unknown }>
  uiLanguage.value = normalizeUiLanguage(customEvent.detail?.language ?? getStoredUiLanguage())
}

const loadGlobalMemory = async () => {
  if (isLoadingGlobalMemory.value) return
  isLoadingGlobalMemory.value = true
  try {
    const entries = await invoke<GlobalMemoryEntry[]>('list_global_memory', { limit: 50 })
    globalMemoryEntries.value = Array.isArray(entries) ? entries : []
  } catch (error) {
    emitToast({
      variant: 'error',
      source: 'global-memory',
      message: `${t.value.loadFailed}${String(error)}`,
    })
  } finally {
    isLoadingGlobalMemory.value = false
  }
}

const saveGlobalMemory = async () => {
  const content = newGlobalMemoryContent.value.trim()
  if (!content || isSavingGlobalMemory.value) return

  isSavingGlobalMemory.value = true
  try {
    await invoke('upsert_global_memory', {
      content,
      kind: newGlobalMemoryKind.value,
      source: 'manual_settings',
    })
    newGlobalMemoryContent.value = ''
    await loadGlobalMemory()
    emitToast({
      variant: 'success',
      source: 'global-memory',
      message: t.value.addDone,
    })
  } catch (error) {
    emitToast({
      variant: 'error',
      source: 'global-memory',
      message: `${t.value.addFailed}${String(error)}`,
    })
  } finally {
    isSavingGlobalMemory.value = false
  }
}

const removeGlobalMemory = async (id: string) => {
  if (isRemovingGlobalMemoryId.value !== null) return
  isRemovingGlobalMemoryId.value = id
  try {
    const removed = await invoke<boolean>('delete_global_memory', { id })
    if (!removed) {
      throw new Error('entry not found')
    }
    globalMemoryEntries.value = globalMemoryEntries.value.filter((entry) => entry.id !== id)
    emitToast({
      variant: 'success',
      source: 'global-memory',
      message: t.value.deleteDone,
    })
  } catch (error) {
    emitToast({
      variant: 'error',
      source: 'global-memory',
      message: `${t.value.deleteFailed}${String(error)}`,
    })
  } finally {
    isRemovingGlobalMemoryId.value = null
  }
}

const clearGlobalMemory = async () => {
  if (isClearingGlobalMemory.value) return

  isClearingGlobalMemory.value = true
  try {
    const clearedCount = await invoke<number>('clear_global_memory')
    globalMemoryEntries.value = []
    emitToast({
      variant: 'success',
      source: 'global-memory',
      message: formatClearDone(clearedCount || 0),
    })
  } catch (error) {
    emitToast({
      variant: 'error',
      source: 'global-memory',
      message: `${t.value.clearFailed}${String(error)}`,
    })
  } finally {
    isClearingGlobalMemory.value = false
    confirmDialogOpen.value = false
  }
}

onMounted(() => {
  window.addEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
  void loadGlobalMemory()
})

onUnmounted(() => {
  window.removeEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
})
</script>

<template>
  <div class="px-6 py-4 flex flex-col h-full overflow-y-auto">
    <ConfirmDialog
      v-model="confirmDialogOpen"
      :title="t.confirmTitle"
      :description="t.confirmDesc"
      :confirm-text="t.confirm"
      :cancel-text="t.cancel"
      :busy="isClearingGlobalMemory"
      destructive
      @confirm="clearGlobalMemory"
    />

    <Card class="mb-3 border-[#ebe9e3] dark:border-[#3b3a37]">
      <CardHeader class="px-4 pb-2">
        <CardTitle class="text-[13.5px] text-[#2a2820] dark:text-[#e8e3db]">{{ t.title }}</CardTitle>
        <CardDescription class="text-[12px]">{{ t.desc }}</CardDescription>
      </CardHeader>
      <CardContent class="px-4 pt-0">
        <div class="space-y-3">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-[12px] text-[#6f6759] dark:text-[#b4aa9c]">{{ t.kindLabel }}</span>
            <Button
              v-for="opt in globalMemoryKindOptions"
              :key="opt.value"
              size="sm"
              :variant="newGlobalMemoryKind === opt.value ? 'default' : 'outline'"
              class="h-8"
              @click="newGlobalMemoryKind = opt.value"
            >
              {{ opt.label }}
            </Button>
          </div>

          <Textarea
            v-model="newGlobalMemoryContent"
            rows="3"
            :placeholder="t.inputPlaceholder"
            class="min-h-20"
          />

          <div class="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              :disabled="isSavingGlobalMemory || !newGlobalMemoryContent.trim()"
              @click="saveGlobalMemory"
            >
              {{ isSavingGlobalMemory ? t.adding : t.addButton }}
            </Button>
            <Button
              variant="outline"
              size="sm"
              :disabled="isClearingGlobalMemory || globalMemoryEntries.length === 0"
              @click="confirmDialogOpen = true"
            >
              {{ isClearingGlobalMemory ? t.clearing : t.clear }}
            </Button>
          </div>

          <div class="rounded-lg border border-[#ece6da] dark:border-[#3b3a37] bg-[#fcfbf9] dark:bg-[#242321]">
            <div v-if="isLoadingGlobalMemory" class="px-3 py-3 text-[12px] text-[#8f8678] dark:text-[#ada496]">
              {{ t.loading }}
            </div>
            <div v-else-if="globalMemoryEntries.length === 0" class="px-3 py-3 text-[12px] text-[#8f8678] dark:text-[#ada496]">
              {{ t.empty }}
            </div>
            <div v-else class="max-h-64 overflow-y-auto divide-y divide-[#ece6da] dark:divide-[#3b3a37]">
              <div
                v-for="entry in globalMemoryEntries"
                :key="entry.id"
                class="flex items-start justify-between gap-3 px-3 py-2.5"
              >
                <div class="min-w-0">
                  <div class="text-[11px] text-[#8f8678] dark:text-[#ada496]">
                    {{ entry.kind }} · {{ t.hits }} {{ entry.hits }}
                  </div>
                  <div class="mt-0.5 text-[12.5px] leading-relaxed text-[#352f25] dark:text-[#e0d8cb] break-words">
                    {{ entry.content }}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-7 px-2 text-[11px] text-[#b94b3c] hover:bg-[#fff1ee] dark:text-[#ef8a7f] dark:hover:bg-[#3b2a2a]"
                  :disabled="isRemovingGlobalMemoryId === entry.id"
                  @click="removeGlobalMemory(entry.id)"
                >
                  {{ t.delete }}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
