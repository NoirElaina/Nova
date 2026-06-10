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

const customModels = ref<Record<string, string[]>>({})
const providerOrder = ref<string[]>([])

const builtinProviderIds = new Set(['anthropic', 'openai'])

const currentProviderId = ref('anthropic')
const providerProfiles = ref<Record<string, ProviderProfile>>({})

const dialogOpen = ref(false)
const dialogDraft = ref<ProviderDraft | null>(null)
const dialogIsNew = ref(false)
const pendingDeleteId = ref<string | null>(null)

const resolveProviderModels = (id: string, profile: ProviderProfile) => {
  if (customModels.value[id]?.length) {
    return customModels.value[id]
  }
  return profile.model ? [profile.model] : []
}

const syncProviderOrder = (profiles: Record<string, ProviderProfile>, order: string[]) => {
  const nextOrder = order.filter((id) => id in profiles)
  for (const id of Object.keys(profiles)) {
    if (!nextOrder.includes(id)) {
      nextOrder.push(id)
    }
  }
  providerOrder.value = nextOrder
}

const compareByProviderOrder = (aId: string, bId: string) => {
  const aIndex = providerOrder.value.indexOf(aId)
  const bIndex = providerOrder.value.indexOf(bId)
  if (aIndex === -1 && bIndex === -1) return aId.localeCompare(bId)
  if (aIndex === -1) return 1
  if (bIndex === -1) return -1
  return aIndex - bIndex
}

const loadSettings = async () => {
  try {
    const settings: any = await invoke('get_settings')
    if (settings) {
      if (settings.providerProfiles && typeof settings.providerProfiles === 'object') {
        providerProfiles.value = settings.providerProfiles
      }
      if (settings.customModels && typeof settings.customModels === 'object') {
        customModels.value = settings.customModels
      }
      if (Array.isArray(settings.providerOrder)) {
        providerOrder.value = settings.providerOrder
      }
      currentProviderId.value = settings.provider || 'anthropic'
      syncProviderOrder(providerProfiles.value, providerOrder.value)
    }
  } catch (error) {
    console.error('Failed to load settings:', error)
  }
}

onMounted(loadSettings)

const saveSettings = async () => {
  try {
    const prevSettings: any = (await invoke('get_settings')) || {}
    syncProviderOrder(providerProfiles.value, providerOrder.value)
    const settings = {
      ...prevSettings,
      provider: currentProviderId.value,
      providerProfiles: providerProfiles.value,
      customModels: customModels.value,
      providerOrder: providerOrder.value,
    }
    await invoke('save_settings', { settings })
    window.dispatchEvent(new CustomEvent('settings-updated'))
  } catch (error) {
    console.error('Failed to save settings:', error)
  }
}

const providersList = computed(() => {
  return Object.entries(providerProfiles.value)
    .map(([id, profile]) => {
      const models = resolveProviderModels(id, profile)

      return {
        id,
        label: profile.displayName || id,
        apiFormat: profile.apiFormat || 'openai',
        model: models.join(' / '),
        models,
        isBuiltin: builtinProviderIds.has(id),
      }
    })
    .sort((a, b) => compareByProviderOrder(a.id, b.id))
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
    models: [...resolveProviderModels(id, profile)],
  }
  dialogIsNew.value = false
  dialogOpen.value = true
}

const handleSaveDraft = async (draft: ProviderDraft, originalId: string | null) => {
  const id = draft.id || 'custom-provider'
  const models = Array.from(new Set((draft.models || []).map((item) => item.trim()).filter(Boolean)))

  if (originalId && originalId !== id) {
    delete providerProfiles.value[originalId]
    delete customModels.value[originalId]
    providerOrder.value = providerOrder.value.map((item) => (item === originalId ? id : item))
  }

  providerProfiles.value[id] = {
    displayName: draft.displayName,
    apiFormat: draft.apiFormat,
    apiKey: draft.apiKey,
    baseUrl: draft.baseUrl,
    model: models[0] || '',
  }

  customModels.value[id] = models

  if (!providerOrder.value.includes(id)) {
    providerOrder.value = [...providerOrder.value, id]
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
  delete customModels.value[id]
  providerOrder.value = providerOrder.value.filter((item) => item !== id)
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
