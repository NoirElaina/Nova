<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Plus } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'

import ProviderCard from './ProviderCard.vue'
import ProviderDialog, { type ProviderDraft } from './ProviderDialog.vue'

type ProviderProfile = {
  displayName?: string
  apiFormat?: 'openai' | 'anthropic' | 'openai_responses' | string
  apiKey: string
  baseUrl: string
  model: string
}

const builtinProviderIds = new Set(['anthropic', 'openai'])

const currentProviderId = ref('anthropic')
const providerProfiles = ref<Record<string, ProviderProfile>>({})

const dialogOpen = ref(false)
const dialogDraft = ref<ProviderDraft | null>(null)
const dialogIsNew = ref(false)
const pendingDeleteId = ref<string | null>(null)

const loadSettings = async () => {
  try {
    const settings: any = await invoke('get_settings')
    if (settings) {
      if (settings.providerProfiles && typeof settings.providerProfiles === 'object') {
        providerProfiles.value = settings.providerProfiles
      }
      currentProviderId.value = settings.provider || 'anthropic'
    }
  } catch (error) {
    console.error('Failed to load settings:', error)
  }
}

onMounted(loadSettings)

const saveSettings = async () => {
  try {
    const prevSettings: any = (await invoke('get_settings')) || {}
    const settings = {
      ...prevSettings,
      provider: currentProviderId.value,
      providerProfiles: providerProfiles.value,
    }
    await invoke('save_settings', { settings })
    window.dispatchEvent(new CustomEvent('settings-updated'))
  } catch (error) {
    console.error('Failed to save settings:', error)
  }
}

const providersList = computed(() => {
  return Object.entries(providerProfiles.value).map(([id, profile]) => ({
    id,
    label: profile.displayName || id,
    apiFormat: profile.apiFormat || 'openai',
    model: profile.model || '',
    isBuiltin: builtinProviderIds.has(id),
  }))
})

const handleSwitch = async (id: string) => {
  currentProviderId.value = id
  await saveSettings()
}

const handleCreate = () => {
  dialogDraft.value = null
  dialogIsNew.value = true
  dialogOpen.value = true
}

const handleEdit = (id: string) => {
  const profile = providerProfiles.value[id]
  if (!profile) return
  dialogDraft.value = {
    id,
    displayName: profile.displayName || id,
    apiFormat: profile.apiFormat || 'openai',
    apiKey: profile.apiKey || '',
    baseUrl: profile.baseUrl || '',
    model: profile.model || ''
  }
  dialogIsNew.value = false
  dialogOpen.value = true
}

const handleSaveDraft = async (draft: ProviderDraft, originalId: string | null) => {
  const id = draft.id || 'custom-provider'
  
  if (originalId && originalId !== id) {
    delete providerProfiles.value[originalId]
  }

  providerProfiles.value[id] = {
    displayName: draft.displayName,
    apiFormat: draft.apiFormat,
    apiKey: draft.apiKey,
    baseUrl: draft.baseUrl,
    model: draft.model
  }

  if (dialogIsNew.value) {
    currentProviderId.value = id
  }

  await saveSettings()
}

const handleDelete = (id: string) => {
  if (builtinProviderIds.has(id)) return
  pendingDeleteId.value = id
}

const confirmDelete = async () => {
  const id = pendingDeleteId.value
  if (!id) return

  delete providerProfiles.value[id]
  if (currentProviderId.value === id) {
    currentProviderId.value = 'openai'
  }
  pendingDeleteId.value = null
  await saveSettings()
}

const deleteDialogOpen = computed({
  get: () => pendingDeleteId.value !== null,
  set: (val) => {
    if (!val) pendingDeleteId.value = null
  }
})

const deleteDialogDesc = computed(() => {
  const id = pendingDeleteId.value
  if (!id) return ''
  const name = providerProfiles.value[id]?.displayName || id
  return `确认删除模型配置 "${name}" 吗？此操作无法撤销。`
})
</script>

<template>
  <div class="flex h-full flex-col px-6 py-6 overflow-y-auto">
    <div class="mb-6 flex items-center justify-between">
      <div class="flex flex-col gap-1">
        <h2 class="text-xl font-bold tracking-tight text-foreground">模型配置</h2>
        <p class="text-sm text-muted-foreground">管理并配置您的 LLM 提供商，这与 cc-switch 风格一致。</p>
      </div>
      <Button @click="handleCreate" class="gap-2">
        <Plus class="h-4 w-4" /> 添加配置
      </Button>
    </div>

    <div class="grid gap-4">
      <ProviderCard
        v-for="provider in providersList"
        :key="provider.id"
        :id="provider.id"
        :label="provider.label"
        :api-format="provider.apiFormat"
        :model="provider.model"
        :is-current="currentProviderId === provider.id"
        @switch="handleSwitch"
        @edit="handleEdit"
        @delete="handleDelete"
      />
    </div>

    <ProviderDialog
      v-model:open="dialogOpen"
      :draft="dialogDraft"
      :is-new="dialogIsNew"
      @save="handleSaveDraft"
    />

    <ConfirmDialog
      v-model="deleteDialogOpen"
      title="删除配置"
      :description="deleteDialogDesc"
      confirm-text="删除"
      cancel-text="取消"
      destructive
      @confirm="confirmDelete"
    />
  </div>
</template>
