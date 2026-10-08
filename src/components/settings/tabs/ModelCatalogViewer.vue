<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  RefreshCw,
  Search,
  Check,
  Copy,
  Layers,
  Sparkles,
  Eye,
  FileText,
  Maximize2
} from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { emitToast } from '@/lib/toast'


interface ModelCatalogSummary {
  id: string
  name: string
  description?: string
  contextLength: number
  maxCompletionTokens?: number
  inputModalities: string[]
  promptPricePerM: string
  completionPricePerM: string
  cacheReadPricePerM?: string
}

interface ModelCatalogStats {
  totalModels: number
  lastUpdatedSecs?: number
  cacheFileExists: boolean
  cacheFilePath: string
}

const models = ref<ModelCatalogSummary[]>([])
const stats = ref<ModelCatalogStats | null>(null)
const loading = ref(false)
const refreshing = ref(false)
const searchQuery = ref('')
const selectedModality = ref<'all' | 'image' | 'text'>('all')
const copiedId = ref<string | null>(null)
let copyTimer: ReturnType<typeof setTimeout> | null = null

const formatTokens = (tokens?: number) => {
  if (!tokens || tokens <= 0) return '-'
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 === 0 ? 0 : 1)}M`
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}k`
  return tokens.toString()
}

const formatLastUpdated = (secs?: number) => {
  if (!secs) return '未同步'
  const date = new Date(secs * 1000)
  return date.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

const loadCatalog = async () => {
  loading.value = true
  try {
    const [fetchedStats, fetchedList] = await Promise.all([
      invoke<ModelCatalogStats>('get_model_catalog_stats'),
      invoke<ModelCatalogSummary[]>('get_model_catalog', { search: null, modality: null }),
    ])
    stats.value = fetchedStats
    models.value = fetchedList
  } catch (error) {
    console.error('Failed to load model catalog:', error)
  } finally {
    loading.value = false
  }
}

const handleRefresh = async () => {
  if (refreshing.value) return
  refreshing.value = true
  try {
    const newStats = await invoke<ModelCatalogStats>('refresh_model_catalog')
    stats.value = newStats
    const updatedList = await invoke<ModelCatalogSummary[]>('get_model_catalog', { search: null, modality: null })
    models.value = updatedList
    emitToast({
      message: `已从 OpenRouter 成功同步 ${updatedList.length} 个最新模型与费率`,
      variant: 'success',
    })
  } catch (error) {
    emitToast({
      message: `从 OpenRouter 同步失败: ${String(error)}`,
      variant: 'error',
    })
  } finally {
    refreshing.value = false
  }
}

const handleCopyId = async (id: string) => {
  try {
    await navigator.clipboard.writeText(id)
    copiedId.value = id
    if (copyTimer) clearTimeout(copyTimer)
    copyTimer = setTimeout(() => {
      copiedId.value = null
    }, 1500)
    emitToast({ message: `已复制模型 ID: ${id}`, variant: 'success' })
  } catch {
    emitToast({ message: '复制失败', variant: 'error' })
  }
}


const filteredModels = computed(() => {
  let list = models.value
  const query = searchQuery.value.trim().toLowerCase()

  if (query) {
    list = list.filter(
      (m) => m.id.toLowerCase().includes(query) || m.name.toLowerCase().includes(query)
    )
  }

  if (selectedModality.value === 'image') {
    list = list.filter((m) =>
      m.inputModalities.some((mod) => mod.toLowerCase() === 'image')
    )
  }

  return list
})

onMounted(loadCatalog)
</script>

<template>
  <div class="flex flex-col gap-5">
    <!-- Header Summary Card -->
    <div class="relative overflow-hidden rounded-xl border border-border/60 bg-card/60 p-5 shadow-sm backdrop-blur-sm">
      <div class="flex flex-wrap items-center justify-between gap-4">
        <div class="flex items-center gap-3">
          <div class="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Sparkles class="h-5 w-5" />
          </div>
          <div>
            <div class="flex items-center gap-2">
              <h3 class="text-base font-semibold text-foreground">OpenRouter 模型数据库</h3>
              <span class="rounded-full bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-500">
                实时动态缓存
              </span>
            </div>
            <p class="text-xs text-muted-foreground mt-0.5">
              已缓存 <span class="font-medium text-foreground">{{ stats?.totalModels ?? models.length }}</span> 个主流模型与计费费率 · 上次同步: {{ formatLastUpdated(stats?.lastUpdatedSecs) }}
            </p>
          </div>
        </div>

        <Button
          variant="outline"
          size="sm"
          class="gap-2 shrink-0 border-primary/20 hover:bg-primary/10"
          :disabled="refreshing"
          @click="handleRefresh"
        >
          <RefreshCw class="h-3.5 w-3.5" :class="{ 'animate-spin': refreshing }" />
          {{ refreshing ? '正在同步...' : '从 OpenRouter 更新' }}
        </Button>
      </div>

      <!-- Search & Filters -->
      <div class="mt-4 flex flex-wrap items-center justify-between gap-3 pt-4 border-t border-border/40">
        <div class="relative flex-1 min-w-[240px] max-w-md">
          <Search class="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
          <input
            v-model="searchQuery"
            type="text"
            placeholder="搜索模型名称或提供商，如 claude, deepseek, gpt, qwen..."
            class="h-9 w-full rounded-lg border border-border/60 bg-background/80 pl-9 pr-3 text-xs text-foreground placeholder:text-muted-foreground focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
          />
        </div>

        <div class="flex items-center gap-1.5 text-xs">
          <button
            type="button"
            class="rounded-md px-2.5 py-1 transition-colors"
            :class="selectedModality === 'all' ? 'bg-primary text-primary-foreground font-medium' : 'bg-muted/50 text-muted-foreground hover:bg-muted'"
            @click="selectedModality = 'all'"
          >
            全部 ({{ models.length }})
          </button>
          <button
            type="button"
            class="flex items-center gap-1 rounded-md px-2.5 py-1 transition-colors"
            :class="selectedModality === 'image' ? 'bg-primary text-primary-foreground font-medium' : 'bg-muted/50 text-muted-foreground hover:bg-muted'"
            @click="selectedModality = 'image'"
          >
            <Eye class="h-3 w-3" />
            支持视觉 (Vision)
          </button>
        </div>
      </div>
    </div>

    <!-- Models Grid -->
    <div v-if="loading && models.length === 0" class="flex h-40 items-center justify-center text-sm text-muted-foreground">
      <RefreshCw class="h-4 w-4 animate-spin mr-2" />
      加载模型目录中...
    </div>

    <div v-else-if="filteredModels.length === 0" class="flex h-32 flex-col items-center justify-center rounded-xl border border-dashed border-border/60 text-xs text-muted-foreground">
      <FileText class="h-6 w-6 mb-2 opacity-50" />
      未找到匹配 "{{ searchQuery }}" 的模型
    </div>

    <div v-else class="grid grid-cols-1 md:grid-cols-2 gap-3 max-h-[520px] overflow-y-auto pr-1">
      <div
        v-for="model in filteredModels"
        :key="model.id"
        class="group relative flex flex-col justify-between rounded-xl border border-border/50 bg-card/40 p-4 transition-all duration-200 hover:border-primary/40 hover:bg-card/70 hover:shadow-sm"
      >
        <div>
          <!-- Title & Badges -->
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0 flex-1">
              <h4 class="text-sm font-semibold text-foreground truncate" :title="model.name">
                {{ model.name }}
              </h4>
              <p class="text-xs text-muted-foreground font-mono truncate select-all" :title="model.id">
                {{ model.id }}
              </p>
            </div>

            <button
              type="button"
              class="flex h-7 w-7 items-center justify-center rounded-md border border-border/60 bg-background/50 text-muted-foreground transition-colors hover:border-primary hover:text-foreground"
              :title="'复制模型 ID: ' + model.id"
              @click="handleCopyId(model.id)"
            >
              <Check v-if="copiedId === model.id" class="h-3.5 w-3.5 text-emerald-500" />
              <Copy v-else class="h-3.5 w-3.5" />
            </button>
          </div>

          <!-- Description if exists -->
          <p v-if="model.description" class="mt-2 text-xs text-muted-foreground line-clamp-2 leading-relaxed">
            {{ model.description }}
          </p>

          <!-- Specs Badges -->
          <div class="mt-3 flex flex-wrap items-center gap-1.5 text-[11px]">
            <span class="inline-flex items-center gap-1 rounded bg-secondary/60 px-2 py-0.5 font-medium text-foreground" title="上下文窗口大小">
              <Layers class="h-3 w-3 text-muted-foreground" />
              上下文 {{ formatTokens(model.contextLength) }}
            </span>

            <span v-if="model.maxCompletionTokens" class="inline-flex items-center gap-1 rounded bg-secondary/60 px-2 py-0.5 font-medium text-foreground" title="单次最大输出 tokens">
              <Maximize2 class="h-3 w-3 text-muted-foreground" />
              输出 {{ formatTokens(model.maxCompletionTokens) }}
            </span>

            <span
              v-if="model.inputModalities.includes('image')"
              class="inline-flex items-center gap-1 rounded bg-blue-500/10 px-2 py-0.5 font-medium text-blue-500"
              title="支持图像输入"
            >
              <Eye class="h-3 w-3" />
              视觉
            </span>
          </div>
        </div>

        <!-- Pricing Row -->
        <div class="mt-4 flex items-center justify-between border-t border-border/40 pt-2.5 text-[11px] text-muted-foreground">
          <div class="flex items-center gap-3">
            <span title="输入 Prompt 每百万 Token 费用">
              输入: <span class="font-medium text-foreground">{{ model.promptPricePerM }} / M</span>
            </span>
            <span title="输出 Completion 每百万 Token 费用">
              输出: <span class="font-medium text-foreground">{{ model.completionPricePerM }} / M</span>
            </span>
          </div>

          <span v-if="model.cacheReadPricePerM" class="text-emerald-500 font-medium" title="KV 缓存命中读取费用">
            缓存: {{ model.cacheReadPricePerM }} / M
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
