<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Eye, EyeOff, Loader2, Plus, RefreshCw, Trash2 } from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { getRawErrorText } from '@/lib/error-display'
import { Checkbox } from '@/components/ui/checkbox'
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

const saveError = ref('')
/** 在线获取的模型列表（类似 CCSwitch）：作为“模型”列下拉候选项。 */
const fetchedModels = ref<{ id: string; ownedBy?: string | null }[]>([])
const fetching = ref(false)
const fetchError = ref('')
/** API Key 明文展示开关 */
const showApiKey = ref(false)
/** 完整填写模式：Base URL 由用户手动填完整地址；
 * 关闭时切换协议会自动补充该协议的官方默认 Base URL。 */
const fullEntry = ref(false)
/** 内置/默认解析值，仅作占位提示 */
const resolvedHints = ref<Record<string, number>>({})

/** 各协议的官方默认 Base URL（后端会自动补齐 /chat/completions 等路径）。 */
const DEFAULT_BASE_URLS: Record<string, string> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com',
  openai_responses: 'https://api.openai.com/v1',
}

const localDraft = ref<ProviderDraft>({
  id: '',
  displayName: '',
  apiFormat: 'openai',
  apiKey: '',
  baseUrl: '',
  models: [],
})

/** 切换协议：非完整填写模式下自动补充默认 Base URL。 */
const handleApiFormatChange = (raw: unknown) => {
  const format = String(raw ?? '')
  localDraft.value.apiFormat = format
  if (fullEntry.value) return
  const defaultUrl = DEFAULT_BASE_URLS[format]
  if (defaultUrl) {
    localDraft.value.baseUrl = defaultUrl
  }
}

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
  fetchedModels.value = []
  fetchError.value = ''
  showApiKey.value = false
  // 编辑既有配置：已填自定义 Base URL 时视为完整填写模式，避免切换协议覆盖它。
  fullEntry.value = !!(newVal && props.draft && props.draft.baseUrl.trim())
  if (newVal) {
    await refreshHints(localDraft.value.models)
  }
})

/** 表格末尾添加一行空模型（模型通过下拉选择）。 */
const addEmptyModelRow = () => {
  localDraft.value.models = [
    ...localDraft.value.models,
    { name: '', contextWindow: null },
  ]
  saveError.value = ''
}

const removeModel = (index: number) => {
  localDraft.value.models = localDraft.value.models.filter((_, itemIndex) => itemIndex !== index)
}

/** 行下拉候选：获取到的模型列表，排除其它行已占用的模型；
 * 当前行的值若不在列表里（未获取/手输）也补进去，保证能显示与选中。 */
const selectOptionsFor = (index: number) => {
  const current = localDraft.value.models[index]?.name.trim() ?? ''
  const exclude = new Set(
    localDraft.value.models.filter((_, i) => i !== index).map((m) => m.name.trim()),
  )
  const options = fetchedModels.value.filter((m) => !exclude.has(m.id))
  if (current && !options.some((m) => m.id === current)) {
    options.unshift({ id: current })
  }
  return options
}

/** 下拉点选：直接替换当前行的模型名。 */
const pickModelOption = async (index: number, raw: unknown) => {
  const item = localDraft.value.models[index]
  const id = String(raw ?? '').trim()
  if (!item || !id) return
  item.name = id
  saveError.value = ''
  if (resolvedHints.value[id] == null) {
    let hint = 200000
    try {
      hint = await invoke<number>('get_model_window_tokens', { model: id })
    } catch {
      // keep default
    }
    resolvedHints.value = { ...resolvedHints.value, [id]: hint }
  }
}

/** 用当前表单里的 API Key / Base URL 直接拉取供应商模型列表。 */
const fetchModels = async () => {
  fetchError.value = ''
  const apiKey = localDraft.value.apiKey.trim()
  const baseUrl = localDraft.value.baseUrl.trim()
  if (!apiKey || !baseUrl) {
    fetchError.value = '请先填写 API Key 和 Base URL'
    return
  }
  fetching.value = true
  try {
    const models = await invoke<{ id: string; ownedBy?: string | null }[]>('fetch_available_models', {
      baseUrl,
      apiKey,
      isFullUrl: fullEntry.value,
      apiFormat: localDraft.value.apiFormat,
    })
    fetchedModels.value = models
    if (models.length === 0) {
      fetchError.value = '接口返回成功，但未包含任何模型'
    }
  } catch (err) {
    fetchedModels.value = []
    fetchError.value = getRawErrorText(err) || '获取模型失败'
  } finally {
    fetching.value = false
  }
}

/** 把获取到的模型一键全部填入表格（上下文窗口走内置库解析）。 */
const addAllFetchedModels = async () => {
  const existing = new Set(localDraft.value.models.map((m) => m.name.trim()))
  const additions: ModelDraftItem[] = []
  for (const item of fetchedModels.value) {
    if (!item.id || existing.has(item.id)) continue
    let hint = 200000
    try {
      hint = await invoke<number>('get_model_window_tokens', { model: item.id })
    } catch {
      // keep default
    }
    resolvedHints.value = { ...resolvedHints.value, [item.id]: hint }
    additions.push({ name: item.id, contextWindow: hint })
  }
  if (additions.length === 0) {
    fetchError.value = '获取到的模型均已在列表中'
    return
  }
  localDraft.value.models = [...localDraft.value.models, ...additions]
  fetchError.value = ''
  saveError.value = ''
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
            <Select :model-value="localDraft.apiFormat" @update:model-value="handleApiFormatChange">
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
            <div class="relative">
              <Input
                id="apiKey"
                v-model="localDraft.apiKey"
                :type="showApiKey ? 'text' : 'password'"
                placeholder="sk-..."
                class="pr-9"
              />
              <button
                type="button"
                class="absolute right-2 top-1/2 -translate-y-1/2 inline-flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:text-foreground"
                :aria-label="showApiKey ? '隐藏 API Key' : '显示 API Key'"
                :title="showApiKey ? '隐藏 API Key' : '显示 API Key'"
                @click="showApiKey = !showApiKey"
              >
                <EyeOff v-if="showApiKey" class="h-4 w-4" />
                <Eye v-else class="h-4 w-4" />
              </button>
            </div>
          </div>

          <div class="grid gap-2">
            <div class="flex items-center justify-between gap-2">
              <Label for="baseUrl">Base URL</Label>
              <label class="flex cursor-pointer select-none items-center gap-1.5 text-xs text-muted-foreground">
                <Checkbox v-model:checked="fullEntry" />
                完整填写
              </label>
            </div>
            <Input
              id="baseUrl"
              v-model="localDraft.baseUrl"
              :placeholder="fullEntry ? 'https://第三方中转的完整地址，例如 https://xxx.com/anthropic/v1/messages' : DEFAULT_BASE_URLS[localDraft.apiFormat] || 'https://api.openai.com/v1'"
            />
            <p class="text-xs text-muted-foreground">
              {{ fullEntry ? '完整填写：手动输入中转/自定义的完整地址。' : '不勾选“完整填写”时，切换协议会自动补充官方默认地址；第三方中转请勾选后手填。' }}
            </p>
          </div>

          <div class="grid gap-2">
            <div class="flex items-center justify-between gap-2">
              <div class="flex items-center gap-2">
                <Label>模型列表</Label>
                <span class="text-xs text-muted-foreground">{{ localDraft.models.length }} 个</span>
              </div>
              <div class="flex items-center gap-2">
                <Button variant="outline" size="sm" class="gap-1.5" :disabled="fetching" @click="fetchModels">
                  <Loader2 v-if="fetching" class="h-3.5 w-3.5 animate-spin" />
                  <RefreshCw v-else class="h-3.5 w-3.5" />
                  {{ fetching ? '获取中…' : '获取模型列表' }}
                </Button>
                <Button variant="outline" size="sm" class="gap-1.5" @click="addEmptyModelRow">
                  <Plus class="h-3.5 w-3.5" /> 添加模型
                </Button>
              </div>
            </div>

            <div v-if="localDraft.models.length > 0" class="grid grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)_2rem] gap-x-2">
              <span class="px-1 pb-1 text-[11px] font-medium text-muted-foreground">模型</span>
              <span class="px-1 pb-1 text-[11px] font-medium text-muted-foreground">上下文窗口</span>
              <span></span>
            </div>

            <div class="max-h-64 overflow-y-auto pr-0.5">
              <div class="flex flex-col gap-2">
                <div
                  v-for="(model, index) in localDraft.models"
                  :key="index"
                  class="grid grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)_2rem] items-center gap-x-2"
                >
                  <Select
                    :model-value="model.name || undefined"
                    @update:model-value="pickModelOption(index, $event)"
                  >
                    <SelectTrigger class="h-9 w-full text-sm">
                      <SelectValue :placeholder="fetchedModels.length > 0 ? '选择模型' : '请先获取模型列表'" />
                    </SelectTrigger>
                    <SelectContent class="max-h-64">
                      <SelectItem
                        v-for="option in selectOptionsFor(index)"
                        :key="option.id"
                        :value="option.id"
                      >
                        {{ option.id }}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                  <Input
                    class="h-9 text-sm tabular-nums"
                    type="number"
                    min="1024"
                    step="1024"
                    :placeholder="contextPlaceholder(model)"
                    :model-value="model.contextWindow ?? ''"
                    @update:model-value="setContextWindow(index, String($event ?? ''))"
                  />
                  <button
                    type="button"
                    class="inline-flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    aria-label="删除模型"
                    @click="removeModel(index)"
                  >
                    <Trash2 class="h-4 w-4" />
                  </button>
                </div>
              </div>
            </div>
            <p
              v-if="localDraft.models.length === 0"
              class="rounded-lg border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground"
            >
              尚未添加模型，点右上“获取模型列表”拉取后在下拉里选择，或点“添加模型”新增一行。
            </p>

            <div v-if="fetchedModels.length > 0" class="flex items-center justify-between gap-2 text-xs text-muted-foreground">
              <span>已获取 {{ fetchedModels.length }} 个可选模型</span>
              <Button variant="ghost" size="sm" class="h-6 px-2 text-xs" @click="addAllFetchedModels">全部添加</Button>
            </div>
            <p v-if="fetchError" class="text-xs text-destructive">{{ fetchError }}</p>

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
