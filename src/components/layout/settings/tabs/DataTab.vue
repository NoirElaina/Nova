<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { emitToast } from '../../../../lib/toast'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import {
  getStoredUiLanguage,
  normalizeUiLanguage,
  type UiLanguage,
} from '../../../../lib/ui-preferences'

type ProviderProfile = {
  apiKey?: string
  baseUrl?: string
  model?: string
  [key: string]: unknown
}

type AppSettings = {
  providerProfiles?: Record<string, ProviderProfile>
  [key: string]: unknown
}

type PendingConfirmAction = 'clear-history' | 'clear-api-keys' | null

const uiLanguage = ref<UiLanguage>(getStoredUiLanguage())
const isClearingHistory = ref(false)
const isClearingApiKeys = ref(false)
const pendingConfirmAction = ref<PendingConfirmAction>(null)

const localeTexts = {
  'zh-CN': {
    introTitle: '数据管理',
    introDesc: '管理本地聊天记录和模型凭证。涉及删除的操作会立即生效，请谨慎执行。',
    historyTitle: '聊天历史',
    historyDesc: '删除所有本地消息、会话记录以及会话级记忆数据。',
    historyButton: '清空历史',
    historyWorking: '清理中...',
    historyConfirm: '确认清空全部聊天历史吗？该操作不可撤销。',
    historyConfirmTitle: '清空全部聊天历史？',
    historyDone: '已清空聊天历史。',
    historyFailed: '清空聊天历史失败：',
    apiTitle: 'API Key',
    apiDesc: '移除当前设置中保存的所有 provider API Key，不会删除 Base URL 和模型列表。',
    apiButton: '移除所有 Key',
    apiWorking: '移除中...',
    apiConfirm: '确认移除所有已保存的 API Key 吗？',
    apiConfirmTitle: '移除所有已保存的 API Key？',
    apiDoneNone: '当前没有可移除的 API Key。',
    apiDoneSome: '已移除 {count} 个已保存的 API Key。',
    apiFailed: '移除 API Key 失败：',
    cancel: '取消',
    confirm: '确认',
  },
  'en-US': {
    introTitle: 'Data Management',
    introDesc: 'Manage local chat history and stored model credentials. Destructive actions take effect immediately.',
    historyTitle: 'Chat History',
    historyDesc: 'Delete all local messages, conversations, and conversation-scoped memory data.',
    historyButton: 'Clear History',
    historyWorking: 'Clearing...',
    historyConfirm: 'Clear all chat history? This action cannot be undone.',
    historyConfirmTitle: 'Clear all chat history?',
    historyDone: 'Chat history cleared.',
    historyFailed: 'Failed to clear chat history: ',
    apiTitle: 'API Keys',
    apiDesc: 'Remove all saved provider API keys from settings. Base URLs and model lists are kept intact.',
    apiButton: 'Remove All Keys',
    apiWorking: 'Removing...',
    apiConfirm: 'Remove all saved API keys?',
    apiConfirmTitle: 'Remove all saved API keys?',
    apiDoneNone: 'No saved API keys were found.',
    apiDoneSome: 'Removed {count} saved API key(s).',
    apiFailed: 'Failed to remove API keys: ',
    cancel: 'Cancel',
    confirm: 'Confirm',
  },
} as const

const t = computed(() => localeTexts[uiLanguage.value])
const confirmDialogOpen = computed({
  get: () => pendingConfirmAction.value !== null,
  set: (value: boolean) => {
    if (!value) {
      pendingConfirmAction.value = null
    }
  },
})
const confirmDialogBusy = computed(() => isClearingHistory.value || isClearingApiKeys.value)
const confirmDialogTitle = computed(() => (
  pendingConfirmAction.value === 'clear-api-keys'
    ? t.value.apiConfirmTitle
    : t.value.historyConfirmTitle
))
const confirmDialogDescription = computed(() => (
  pendingConfirmAction.value === 'clear-api-keys'
    ? t.value.apiConfirm
    : t.value.historyConfirm
))

const formatApiDoneMessage = (count: number) => (
  count > 0
    ? t.value.apiDoneSome.replace('{count}', String(count))
    : t.value.apiDoneNone
)

const requestClearHistory = () => {
  if (isClearingHistory.value) return
  pendingConfirmAction.value = 'clear-history'
}

const requestClearApiKeys = () => {
  if (isClearingApiKeys.value) return
  pendingConfirmAction.value = 'clear-api-keys'
}

const handleUiLanguageUpdated = (event: Event) => {
  const customEvent = event as CustomEvent<{ language?: unknown }>
  uiLanguage.value = normalizeUiLanguage(customEvent.detail?.language ?? getStoredUiLanguage())
}

const clearHistory = async () => {
  if (isClearingHistory.value) {
    return
  }

  isClearingHistory.value = true
  try {
    await invoke('clear_history', { conversationId: null })
    window.dispatchEvent(new CustomEvent('history-cleared'))
    emitToast({
      variant: 'success',
      source: 'history',
      message: t.value.historyDone,
    })
  } catch (error) {
    console.error('Failed to clear history:', error)
  } finally {
    isClearingHistory.value = false
    pendingConfirmAction.value = null
  }
}

const clearApiKeys = async () => {
  if (isClearingApiKeys.value) {
    return
  }

  isClearingApiKeys.value = true
  try {
    const settings = ((await invoke('get_settings')) || {}) as AppSettings
    let cleared = 0

    const profiles = settings.providerProfiles && typeof settings.providerProfiles === 'object'
      ? settings.providerProfiles
      : {}

    const nextProfiles = Object.fromEntries(
      Object.entries(profiles).map(([provider, profile]) => {
        const nextProfile: ProviderProfile = { ...profile }
        if (typeof nextProfile.apiKey === 'string' && nextProfile.apiKey.trim()) {
          cleared += 1
        }
        nextProfile.apiKey = ''
        return [provider, nextProfile]
      }),
    )

    const nextSettings: AppSettings = {
      ...settings,
      providerProfiles: nextProfiles,
    }

    await invoke('save_settings', { settings: nextSettings })
    window.dispatchEvent(new CustomEvent('settings-updated'))
    emitToast({
      variant: 'success',
      source: 'settings',
      message: formatApiDoneMessage(cleared),
    })
  } catch (error) {
    console.error('Failed to clear API keys:', error)
  } finally {
    isClearingApiKeys.value = false
    pendingConfirmAction.value = null
  }
}

const handleConfirmAction = () => {
  if (pendingConfirmAction.value === 'clear-api-keys') {
    void clearApiKeys()
    return
  }

  if (pendingConfirmAction.value === 'clear-history') {
    void clearHistory()
  }
}

onMounted(() => {
  window.addEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
})

onUnmounted(() => {
  window.removeEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
})
</script>

<template>
  <div class="px-6 py-4 flex flex-col h-full overflow-y-auto">
    <ConfirmDialog
      v-model="confirmDialogOpen"
      :title="confirmDialogTitle"
      :description="confirmDialogDescription"
      :confirm-text="t.confirm"
      :cancel-text="t.cancel"
      :busy="confirmDialogBusy"
      destructive
      @confirm="handleConfirmAction"
    />

    <Card class="mb-3 border-[#e5e7eb] bg-[#f9fafb] dark:border-[#333] dark:bg-[#1e1e1e]">
      <CardHeader class="px-4 pb-2">
        <CardTitle class="text-[13.5px] text-[#111827] dark:text-[#f3f4f6]">{{ t.introTitle }}</CardTitle>
        <CardDescription class="text-[12px] leading-relaxed">{{ t.introDesc }}</CardDescription>
      </CardHeader>
    </Card>

    <Card class="mb-3 border-[#e5e7eb] dark:border-[#333]">
      <CardHeader class="px-4 pb-2">
        <CardTitle class="text-[13.5px] text-[#111827] dark:text-[#f3f4f6]">{{ t.historyTitle }}</CardTitle>
        <CardDescription class="text-[12px]">{{ t.historyDesc }}</CardDescription>
      </CardHeader>
      <CardContent class="px-4 pt-0">
        <Button
          variant="destructive"
          size="sm"
          :disabled="isClearingHistory"
          @click="requestClearHistory"
        >
          {{ isClearingHistory ? t.historyWorking : t.historyButton }}
        </Button>
      </CardContent>
    </Card>

    <Card class="mb-3 border-[#e5e7eb] dark:border-[#333]">
      <CardHeader class="px-4 pb-2">
        <CardTitle class="text-[13.5px] text-[#111827] dark:text-[#f3f4f6]">{{ t.apiTitle }}</CardTitle>
        <CardDescription class="text-[12px]">{{ t.apiDesc }}</CardDescription>
      </CardHeader>
      <CardContent class="px-4 pt-0">
        <Button
          variant="outline"
          size="sm"
          class="border-red-200 text-red-600 hover:bg-red-50 dark:border-red-900/50 dark:text-red-400 dark:hover:bg-red-950/30"
          :disabled="isClearingApiKeys"
          @click="requestClearApiKeys"
        >
          {{ isClearingApiKeys ? t.apiWorking : t.apiButton }}
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
