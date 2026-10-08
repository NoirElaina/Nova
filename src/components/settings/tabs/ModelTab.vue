<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'

import { Plus, Cpu, Sparkles } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'

import ProviderCard from './ProviderCard.vue'
import ProviderDialog, { type ProviderDraft, type ModelDraftItem } from './ProviderDialog.vue'
import ModelCatalogViewer from './ModelCatalogViewer.vue'

const currentSubTab = ref<'providers' | 'catalog'>('providers')


type ProviderProfile = {
  displayName?: string
  apiFormat?: 'openai' | 'anthropic' | 'openai_responses' | string
  apiKey: string
  baseUrl: string
  model: string
}

const customModels = ref<Record<string, string[]>>({})
/** model name -> context window tokens */
const modelContextWindows = ref<Record<string, number>>({})
const providerOrder = ref<string[]>([])

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

const toModelDraftItems = (names: string[]): ModelDraftItem[] =>
  names.map((name) => ({
    name,
    contextWindow:
      typeof modelContextWindows.value[name] === 'number' && modelContextWindows.value[name] > 0
        ? modelContextWindows.value[name]
        : null,
  }))

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
      if (settings.modelContextWindows && typeof settings.modelContextWindows === 'object') {
        const next: Record<string, number> = {}
        for (const [key, value] of Object.entries(settings.modelContextWindows)) {
          const n = Number(value)
          if (key.trim() && Number.isFinite(n) && n > 0) {
            next[key.trim()] = Math.round(n)
          }
        }
        modelContextWindows.value = next
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
      modelContextWindows: modelContextWindows.value,
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
    models: toModelDraftItems([...resolveProviderModels(id, profile)]),
  }
  dialogIsNew.value = false
  dialogOpen.value = true
}

const handleSaveDraft = async (draft: ProviderDraft, originalId: string | null) => {
  const id = draft.id || 'custom-provider'
  const modelItems = draft.models || []
  const models = modelItems.map((item) => item.name.trim()).filter(Boolean)

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

  // 合并本 provider 模型的上下文覆盖；清除本批中显式留空的项
  const nextWindows = { ...modelContextWindows.value }
  for (const item of modelItems) {
    const name = item.name.trim()
    if (!name) continue
    if (typeof item.contextWindow === 'number' && item.contextWindow > 0) {
      nextWindows[name] = Math.round(item.contextWindow)
    } else {
      delete nextWindows[name]
    }
  }
  modelContextWindows.value = nextWindows

  if (!providerOrder.value.includes(id)) {
    providerOrder.value = [...providerOrder.value, id]
  }

  if (dialogIsNew.value) {
    currentProviderId.value = id
  }

  await saveSettings()
}

const handleDelete = (id: string) => {
  // 所有提供商（含 anthropic/openai）均可删除，确认弹窗后执行。
  pendingDeleteId.value = id
}

const confirmDelete = async () => {
  const id = pendingDeleteId.value
  if (!id) return

  delete providerProfiles.value[id]
  delete customModels.value[id]
  providerOrder.value = providerOrder.value.filter((item) => item !== id)
  if (currentProviderId.value === id) {
    // 回落到剩余的第一个提供商；全部删完时后端会重建默认空配置。
    currentProviderId.value = providerOrder.value[0] ?? 'anthropic'
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
    <!-- Sub-tab pill selector -->
    <div class="mb-6 flex items-center justify-between border-b border-border/40 pb-4">
      <div class="flex items-center gap-2">
        <button
          type="button"
          class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-lg font-medium transition-all"
          :class="currentSubTab === 'providers' ? 'bg-primary text-primary-foreground shadow-sm' : 'bg-muted/50 text-muted-foreground hover:bg-muted hover:text-foreground'"
          @click="currentSubTab = 'providers'"
        >
          <Cpu class="h-3.5 w-3.5" />
          服务商配置
        </button>

        <button
          type="button"
          class="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-lg font-medium transition-all"
          :class="currentSubTab === 'catalog' ? 'bg-primary text-primary-foreground shadow-sm' : 'bg-muted/50 text-muted-foreground hover:bg-muted hover:text-foreground'"
          @click="currentSubTab = 'catalog'"
        >
          <Sparkles class="h-3.5 w-3.5" />
          OpenRouter 模型库 (300+)
        </button>
      </div>

      <Button v-if="currentSubTab === 'providers'" @click="handleCreate" size="sm" class="gap-1.5">
        <Plus class="h-3.5 w-3.5" /> 添加配置
      </Button>
    </div>

    <!-- Providers Tab -->
    <template v-if="currentSubTab === 'providers'">
      <div class="mb-4">
        <p class="text-xs text-muted-foreground">
          管理大模型 API 连接、密钥与上下文窗口设置。
        </p>
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
    </template>

    <!-- OpenRouter Model Catalog Tab -->
    <template v-else>
      <ModelCatalogViewer />
    </template>


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
