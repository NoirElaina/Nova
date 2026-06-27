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

const uiLanguage = ref<UiLanguage>(getStoredUiLanguage())
const isLoadingMemory = ref(false)
const isSavingMemory = ref(false)
const isClearingMemory = ref(false)
const removingEntry = ref<string | null>(null)
const confirmDialogOpen = ref(false)
const memoryEntries = ref<string[]>([])
const newMemoryContent = ref('')

const localeTexts = {
  'zh-CN': {
    title: '全局记忆',
    desc: '跨会话长期保留的偏好与事实。AI 会自动写入；你也可以在这里手动管理。',
    inputPlaceholder: '例如：默认用中文回复；代码修改优先最小化变更。',
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
    clearDone: '已清空全局记忆。',
    clearFailed: '清空全局记忆失败：',
    cancel: '取消',
    confirm: '确认',
    loading: '加载中...',
  },
  'en-US': {
    title: 'Global Memory',
    desc: 'Persistent cross-session preferences and facts. AI writes them automatically, and you can manage them here.',
    inputPlaceholder: 'Example: Reply in Chinese by default; prefer minimal code changes.',
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
    clearDone: 'Global memory cleared.',
    clearFailed: 'Failed to clear global memory: ',
    cancel: 'Cancel',
    confirm: 'Confirm',
    loading: 'Loading...',
  },
} as const

const t = computed(() => localeTexts[uiLanguage.value])

const handleUiLanguageUpdated = (event: Event) => {
  const customEvent = event as CustomEvent<{ language?: unknown }>
  uiLanguage.value = normalizeUiLanguage(customEvent.detail?.language ?? getStoredUiLanguage())
}

const loadMemory = async () => {
  if (isLoadingMemory.value) return
  isLoadingMemory.value = true
  try {
    const entries = await invoke<string[]>('list_memory_entries')
    memoryEntries.value = Array.isArray(entries) ? entries : []
  } catch (error) {
    console.error(t.value.loadFailed, error)
  } finally {
    isLoadingMemory.value = false
  }
}

const saveMemory = async () => {
  const content = newMemoryContent.value.trim()
  if (!content || isSavingMemory.value) return

  isSavingMemory.value = true
  try {
    await invoke('add_memory_entry', { content })
    newMemoryContent.value = ''
    await loadMemory()
    emitToast({
      variant: 'success',
      source: 'global-memory',
      message: t.value.addDone,
    })
  } catch (error) {
    console.error(t.value.addFailed, error)
  } finally {
    isSavingMemory.value = false
  }
}

const removeMemory = async (content: string) => {
  if (removingEntry.value !== null) return
  removingEntry.value = content
  try {
    await invoke('remove_memory_entry', { oldText: content })
    memoryEntries.value = memoryEntries.value.filter((entry) => entry !== content)
    emitToast({
      variant: 'success',
      source: 'global-memory',
      message: t.value.deleteDone,
    })
  } catch (error) {
    console.error(t.value.deleteFailed, error)
  } finally {
    removingEntry.value = null
  }
}

const clearMemory = async () => {
  if (isClearingMemory.value) return

  isClearingMemory.value = true
  try {
    await invoke('clear_memory_entries')
    memoryEntries.value = []
    emitToast({
      variant: 'success',
      source: 'global-memory',
      message: t.value.clearDone,
    })
  } catch (error) {
    console.error(t.value.clearFailed, error)
  } finally {
    isClearingMemory.value = false
    confirmDialogOpen.value = false
  }
}

onMounted(() => {
  window.addEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
  void loadMemory()
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
      :busy="isClearingMemory"
      destructive
      @confirm="clearMemory"
    />

    <Card class="mb-3 border-[#e5e7eb] dark:border-[#333]">
      <CardHeader class="px-4 pb-2">
        <CardTitle class="text-[13.5px] text-[#111827] dark:text-[#f3f4f6]">{{ t.title }}</CardTitle>
        <CardDescription class="text-[12px]">{{ t.desc }}</CardDescription>
      </CardHeader>
      <CardContent class="px-4 pt-0">
        <div class="space-y-3">
          <Textarea
            v-model="newMemoryContent"
            rows="3"
            :placeholder="t.inputPlaceholder"
            class="min-h-20"
          />

          <div class="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              :disabled="isSavingMemory || !newMemoryContent.trim()"
              @click="saveMemory"
            >
              {{ isSavingMemory ? t.adding : t.addButton }}
            </Button>
            <Button
              variant="outline"
              size="sm"
              :disabled="isClearingMemory || memoryEntries.length === 0"
              @click="confirmDialogOpen = true"
            >
              {{ isClearingMemory ? t.clearing : t.clear }}
            </Button>
          </div>

          <div class="rounded-lg border border-[#e5e7eb] dark:border-[#333] bg-[#f9fafb] dark:bg-[#1e1e1e]">
            <div v-if="isLoadingMemory" class="px-3 py-3 text-[12px] text-[#64748b] dark:text-[#a3a3a3]">
              {{ t.loading }}
            </div>
            <div v-else-if="memoryEntries.length === 0" class="px-3 py-3 text-[12px] text-[#64748b] dark:text-[#a3a3a3]">
              {{ t.empty }}
            </div>
            <div v-else class="max-h-64 overflow-y-auto divide-y divide-[#e5e7eb] dark:divide-[#333]">
              <div
                v-for="entry in memoryEntries"
                :key="entry"
                class="flex items-start justify-between gap-3 px-3 py-2.5"
              >
                <div class="mt-0.5 text-[12.5px] leading-relaxed text-[#111827] dark:text-[#ececec] break-words">
                  {{ entry }}
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-7 px-2 text-[11px] text-red-500 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-950/30"
                  :disabled="removingEntry === entry"
                  @click="removeMemory(entry)"
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
