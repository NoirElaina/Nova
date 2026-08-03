<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { X } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

export type ModelDraftItem = {
  name: string
  /** 用户配置的上下文窗口；空/未填表示跟随内置 JSON 或默认 */
  contextWindow: number | null
}

export interface ProviderDraft {
  id: string
  displayName: string
  apiFormat: string
  apiKey: string
  baseUrl: string
  models: ModelDraftItem[]
}

const props = defineProps<{
  open: boolean
  draft: ProviderDraft | null
  isNew: boolean
}>()

const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'save', draft: ProviderDraft, originalId: string | null): void
}>()

const newModelInput = ref('')
const saveError = ref('')
/** 内置/默认解析值，仅作占位提示 */
const resolvedHints = ref<Record<string, number>>({})

const localDraft = ref<ProviderDraft>({
  id: '',
  displayName: '',
  apiFormat: 'openai',
  apiKey: '',
  baseUrl: '',
  models: [],
})

const normalizeModels = (models: ModelDraftItem[]): ModelDraftItem[] => {
  const seen = new Set<string>()
  const next: ModelDraftItem[] = []
  for (const item of models) {
    const name = (item?.name ?? '').trim()
    if (!name || seen.has(name)) continue
    seen.add(name)
    const raw = item.contextWindow
    const contextWindow =
      typeof raw === 'number' && Number.isFinite(raw) && raw > 0
        ? Math.round(raw)
        : null
    next.push({ name, contextWindow })
  }
  return next
}

const refreshHints = async (models: ModelDraftItem[]) => {
  const next: Record<string, number> = { ...resolvedHints.value }
  await Promise.all(
    models.map(async (item) => {
      if (next[item.name] != null) return
      try {
        next[item.name] = await invoke<number>('get_model_window_tokens', {
          model: item.name,
        })
      } catch {
        next[item.name] = 200000
      }
    }),
  )
  resolvedHints.value = next
}

watch(() => props.open, async (newVal) => {
  saveError.value = ''
  if (newVal && props.draft) {
    localDraft.value = {
      ...props.draft,
      models: normalizeModels(props.draft.models),
    }
  } else if (newVal && !props.draft) {
    localDraft.value = {
      id: '',
      displayName: '',
      apiFormat: 'openai',
      apiKey: '',
      baseUrl: '',
      models: [],
    }
  }
  newModelInput.value = ''
  if (newVal) {
    await refreshHints(localDraft.value.models)
  }
})

const addModel = async () => {
  const value = newModelInput.value.trim()
  if (!value) return
  if (!localDraft.value.models.some((item) => item.name === value)) {
    let hint = 200000
    try {
      hint = await invoke<number>('get_model_window_tokens', { model: value })
    } catch {
      // keep default
    }
    resolvedHints.value = { ...resolvedHints.value, [value]: hint }
    localDraft.value.models = [
      ...localDraft.value.models,
      { name: value, contextWindow: hint },
    ]
  }
  newModelInput.value = ''
  saveError.value = ''
}

const removeModel = (index: number) => {
  localDraft.value.models = localDraft.value.models.filter((_, itemIndex) => itemIndex !== index)
}

const setContextWindow = (index: number, raw: string) => {
  const item = localDraft.value.models[index]
  if (!item) return
  const trimmed = raw.trim()
  if (!trimmed) {
    item.contextWindow = null
    return
  }
  const parsed = Number.parseInt(trimmed.replace(/[,_\s]/g, ''), 10)
  item.contextWindow = Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

const contextPlaceholder = (model: ModelDraftItem) => {
  const hint = resolvedHints.value[model.name]
  return hint ? `默认 ${hint.toLocaleString()}` : '例如 200000'
}

const handleSave = () => {
  if (!localDraft.value.id.trim()) {
    localDraft.value.id = localDraft.value.displayName.toLowerCase().replace(/[^a-z0-9]/g, '-')
  }

  const models = normalizeModels(localDraft.value.models)
  if (models.length === 0) {
    saveError.value = '请至少添加一个模型'
    return
  }

  localDraft.value.models = models
  emit('save', localDraft.value, props.draft?.id || null)
  emit('update:open', false)
}
</script>

<template>
  <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-sm">
    <div class="flex max-h-[min(90vh,720px)] w-full max-w-2xl flex-col rounded-xl border bg-background shadow-lg">
      <div class="shrink-0 border-b px-6 py-5">
        <h2 class="text-xl font-bold tracking-tight">{{ isNew ? '添加模型配置' : '编辑模型配置' }}</h2>
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto px-6 py-4">
        <div class="grid gap-4">
          <div class="grid gap-2">
            <Label for="displayName">显示名称</Label>
            <Input id="displayName" v-model="localDraft.displayName" placeholder="例如: OpenAI" />
          </div>

          <div v-if="isNew" class="grid gap-2">
            <Label for="id">内部标识符 (ID)</Label>
            <Input id="id" v-model="localDraft.id" placeholder="留空则自动生成" />
          </div>

          <div class="grid gap-2">
            <Label>接口协议格式 (API Format)</Label>
            <Select v-model="localDraft.apiFormat">
              <SelectTrigger>
                <SelectValue placeholder="选择协议" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="openai">OpenAI 兼容</SelectItem>
                <SelectItem value="anthropic">Anthropic 兼容</SelectItem>
                <SelectItem value="openai_responses">OpenAI Responses (O1模型)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div class="grid gap-2">
            <Label for="apiKey">API Key</Label>
            <Input id="apiKey" v-model="localDraft.apiKey" type="password" placeholder="sk-..." />
          </div>

          <div class="grid gap-2">
            <Label for="baseUrl">Base URL</Label>
            <Input id="baseUrl" v-model="localDraft.baseUrl" placeholder="https://api.openai.com/v1" />
          </div>

          <div class="grid gap-2">
            <div class="flex items-center justify-between gap-2">
              <Label>模型列表</Label>
              <span class="text-xs text-muted-foreground">{{ localDraft.models.length }} 个</span>
            </div>

            <div
              v-if="localDraft.models.length > 0"
              class="max-h-56 overflow-y-auto rounded-lg border border-border bg-muted/30 p-2"
            >
              <div class="flex flex-col gap-2">
                <div
                  v-for="(model, index) in localDraft.models"
                  :key="`${model.name}-${index}`"
                  class="flex flex-wrap items-center gap-2 rounded-md border border-border bg-background px-2.5 py-2"
                >
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-1.5">
                      <span class="truncate text-sm font-medium" :title="model.name">{{ model.name }}</span>
                      <span v-if="index === 0" class="shrink-0 text-[10px] text-muted-foreground">默认</span>
                    </div>
                  </div>
                  <div class="flex items-center gap-1.5">
                    <Label :for="`ctx-${index}`" class="shrink-0 text-[11px] text-muted-foreground whitespace-nowrap">
                      上下文
                    </Label>
                    <Input
                      :id="`ctx-${index}`"
                      class="h-8 w-[7.5rem] text-xs tabular-nums"
                      type="number"
                      min="1024"
                      step="1024"
                      :placeholder="contextPlaceholder(model)"
                      :model-value="model.contextWindow ?? ''"
                      @update:model-value="setContextWindow(index, String($event ?? ''))"
                    />
                    <span class="text-[10px] text-muted-foreground">tokens</span>
                  </div>
                  <button
                    type="button"
                    class="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="删除模型"
                    @click="removeModel(index)"
                  >
                    <X class="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            </div>
            <p v-else class="rounded-lg border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground">
              尚未添加模型，请在下方输入后点击添加
            </p>

            <div class="flex items-center gap-2">
              <Input
                v-model="newModelInput"
                placeholder="输入模型名，Enter 添加"
                @keydown.enter.prevent="addModel"
              />
              <Button variant="outline" class="shrink-0" @click="addModel">添加</Button>
            </div>
            <p class="text-xs text-muted-foreground">
              列表第一项为聊天区默认模型。上下文窗口优先用此处配置；留空则查内置库，仍无则 200K。
            </p>
            <p v-if="saveError" class="text-xs text-destructive">{{ saveError }}</p>
          </div>
        </div>
      </div>

      <div class="flex shrink-0 justify-end gap-3 border-t px-6 py-4">
        <Button variant="outline" @click="emit('update:open', false)">取消</Button>
        <Button @click="handleSave">保存</Button>
      </div>
    </div>
  </div>
</template>
