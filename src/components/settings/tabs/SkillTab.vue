<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import {
  BookOpen,
  Check,
  Code2,
  Copy,
  FolderOpen,
  Maximize2,
  Minimize2,
  RotateCw,
  Sparkles,
  Trash2,
  X,
  ChevronDown,
  ChevronRight,
  FileText,
} from 'lucide-vue-next'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import MarkdownRenderer from '@/components/chat/MarkdownRenderer.vue'

const deletingPath = ref<string | null>(null)
const deleteError = ref('')
const pendingDeleteSkill = ref<SkillItem | null>(null)

const deleteDialogOpen = computed({
  get: () => pendingDeleteSkill.value !== null,
  set: (value: boolean) => {
    if (!value && !deletingPath.value) pendingDeleteSkill.value = null
  },
})
const deleteDialogDescription = computed(() =>
  pendingDeleteSkill.value
    ? `确定要删除技能「${pendingDeleteSkill.value.name}」吗？此操作将永久删除该技能目录，无法恢复。`
    : '',
)

type SkillItem = {
  name: string
  description: string
  path: string
  enabled: boolean
}

const loading = ref(false)
const saving = ref(false)
const savedTip = ref(false)
const error = ref('')
const skills = ref<SkillItem[]>([])
const rawSettings = ref<any>({})

// 查看详情弹窗状态
const viewingSkill = ref<SkillItem | null>(null)
const viewingContent = ref('')
const viewingLoading = ref(false)
const viewingError = ref('')
const viewingMode = ref<'preview' | 'source'>('preview')
const isMaximized = ref(false)
const copied = ref(false)
const pathCopied = ref(false)
const showRawFrontmatter = ref(false)

const normalize = (name: string) => name.trim().toLowerCase()

const refresh = async () => {
  loading.value = true
  error.value = ''
  try {
    const settings: any = (await invoke('get_settings')) || {}
    rawSettings.value = settings
    const disabled = new Set<string>(
      (Array.isArray(settings.disabledSkills) ? settings.disabledSkills : [])
        .filter((v: unknown) => typeof v === 'string')
        .map((v: string) => normalize(v))
    )

    const list = await invoke<Array<{ name: string; description: string; path: string }>>('list_skills')
    skills.value = list.map((s) => ({
      ...s,
      enabled: !disabled.has(normalize(s.name)),
    }))
  } catch (e) {
    console.error('Failed to load skills:', e)
    skills.value = []
  } finally {
    loading.value = false
  }
}

const setAllEnabled = (enabled: boolean) => {
  skills.value = skills.value.map((s) => ({ ...s, enabled }))
  schedulePersist()
}

const toggleSkill = (skill: SkillItem) => {
  skill.enabled = !skill.enabled
  schedulePersist()
}

const openViewSkill = async (skill: SkillItem) => {
  viewingSkill.value = skill
  viewingContent.value = ''
  viewingLoading.value = true
  viewingError.value = ''
  viewingMode.value = 'preview'
  isMaximized.value = false
  copied.value = false
  pathCopied.value = false
  showRawFrontmatter.value = false
  try {
    const content = await invoke<string>('read_skill_content', { path: skill.path })
    viewingContent.value = content
  } catch (err) {
    viewingError.value = typeof err === 'string' ? err : '读取技能文件内容失败'
  } finally {
    viewingLoading.value = false
  }
}

const closeViewSkill = () => {
  viewingSkill.value = null
  viewingContent.value = ''
}

const copySkillContent = async () => {
  if (!viewingContent.value) return
  try {
    await navigator.clipboard.writeText(viewingContent.value)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch (err) {
    console.error('Failed to copy skill content:', err)
  }
}

const copySkillPath = async () => {
  if (!viewingSkill.value) return
  try {
    await navigator.clipboard.writeText(viewingSkill.value.path)
    pathCopied.value = true
    setTimeout(() => {
      pathCopied.value = false
    }, 2000)
  } catch (err) {
    console.error('Failed to copy path:', err)
  }
}

const openInExplorer = async () => {
  if (!viewingSkill.value) return
  try {
    await revealItemInDir(viewingSkill.value.path)
  } catch (err) {
    console.error('Failed to reveal file in explorer:', err)
  }
}

/** 结构化解析技能 Markdown 内容及 YAML Frontmatter */
interface ParsedSkillDoc {
  hasFrontmatter: boolean
  frontmatter: Record<string, string>
  frontmatterEntries: Array<{ key: string; value: string }>
  rawFrontmatter: string
  body: string
  lineCount: number
  byteSize: number
}

const parsedSkillDoc = computed<ParsedSkillDoc>(() => {
  const raw = viewingContent.value || ''
  const lineCount = raw ? raw.split(/\r?\n/).length : 0
  const byteSize = new TextEncoder().encode(raw).length

  if (!raw.trim()) {
    return {
      hasFrontmatter: false,
      frontmatter: {},
      frontmatterEntries: [],
      rawFrontmatter: '',
      body: '',
      lineCount: 0,
      byteSize: 0,
    }
  }

  const trimmed = raw.trimStart()
  if (trimmed.startsWith('---')) {
    const match = raw.match(/^---[ \t]*\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)([\s\S]*)$/)
    if (match) {
      const rawFm = match[1]
      const body = match[2]
      const frontmatter: Record<string, string> = {}
      const frontmatterEntries: Array<{ key: string; value: string }> = []

      let currentKey = ''
      let currentValue = ''
      for (const line of rawFm.split(/\r?\n/)) {
        const colonIdx = line.indexOf(':')
        if (colonIdx !== -1 && !/^\s/.test(line)) {
          if (currentKey) {
            const val = currentValue.trim()
            frontmatter[currentKey] = val
            frontmatterEntries.push({ key: currentKey, value: val })
          }
          currentKey = line.slice(0, colonIdx).trim()
          currentValue = line.slice(colonIdx + 1).trim()
        } else if (currentKey) {
          currentValue += (currentValue ? '\n' : '') + line.trim()
        }
      }
      if (currentKey) {
        const val = currentValue.trim()
        frontmatter[currentKey] = val
        frontmatterEntries.push({ key: currentKey, value: val })
      }

      return {
        hasFrontmatter: true,
        frontmatter,
        frontmatterEntries,
        rawFrontmatter: rawFm,
        body: body.trim(),
        lineCount,
        byteSize,
      }
    }
  }

  return {
    hasFrontmatter: false,
    frontmatter: {},
    frontmatterEntries: [],
    rawFrontmatter: '',
    body: raw,
    lineCount,
    byteSize,
  }
})

// 源码模式代码行拆分
const sourceLines = computed(() => {
  if (!viewingContent.value) return []
  return viewingContent.value.split(/\r?\n/)
})

// 格式化文件大小
const formatFileSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

/** 立即持久化当前停用列表 */
const persistDisabledSkills = async () => {
  saving.value = true
  error.value = ''
  try {
    const currentDisabled = skills.value.filter((s) => !s.enabled).map((s) => s.name)

    const latest: any = (await invoke('get_settings')) || {}
    const settings = {
      ...latest,
      disabledSkills: currentDisabled,
    }

    await invoke('save_settings', { settings })
    rawSettings.value = settings
    window.dispatchEvent(new CustomEvent('settings-updated'))
    savedTip.value = true
    setTimeout(() => (savedTip.value = false), 2000)
  } catch (e) {
    error.value = '保存停用状态失败，请重试'
    console.error('Failed to save skill settings:', e)
  } finally {
    saving.value = false
  }
}

/** 防抖合并连续开关 */
let persistTimer: ReturnType<typeof setTimeout> | null = null
const schedulePersist = () => {
  if (persistTimer) clearTimeout(persistTimer)
  persistTimer = setTimeout(() => {
    persistTimer = null
    void persistDisabledSkills()
  }, 300)
}

onMounted(refresh)

onBeforeUnmount(() => {
  if (persistTimer) {
    clearTimeout(persistTimer)
    persistTimer = null
    void persistDisabledSkills()
  }
})

const requestDeleteSkill = (skill: SkillItem) => {
  if (deletingPath.value) return
  deleteError.value = ''
  pendingDeleteSkill.value = skill
}

const confirmDeleteSkill = async () => {
  const skill = pendingDeleteSkill.value
  if (!skill || deletingPath.value) return
  deletingPath.value = skill.path
  deleteError.value = ''
  try {
    await invoke('delete_skill', { path: skill.path })
    await refresh()
  } catch (e) {
    deleteError.value = `删除技能失败：${e}`
    console.error(`Failed to delete skill (${skill.path}):`, e)
  } finally {
    deletingPath.value = null
    pendingDeleteSkill.value = null
  }
}
</script>

<template>
  <div class="px-6 py-4 flex flex-col h-full overflow-y-auto">
    <ConfirmDialog
      v-model="deleteDialogOpen"
      title="删除技能"
      :description="deleteDialogDescription"
      confirm-text="删除"
      cancel-text="取消"
      :busy="deletingPath !== null"
      destructive
      @confirm="confirmDeleteSkill"
    />

    <div class="mb-4 flex items-center justify-between">
      <span class="text-[12.5px] text-[#64748b] dark:text-[#a3a3a3]">{{ skills.length }} 个技能</span>
      <div class="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          class="border-[#d1d5db] text-[#475569] hover:bg-[#f3f4f6] dark:border-[#444] dark:text-[#a3a3a3] dark:hover:bg-[#2a2a2a] gap-1.5"
          :disabled="loading"
          @click="refresh"
        >
          <RotateCw class="w-3.5 h-3.5" :class="{ 'animate-spin': loading }" />
          刷新
        </Button>
        <Button
          variant="outline"
          size="sm"
          class="border-[#d1d5db] text-[#475569] hover:bg-[#f3f4f6] dark:border-[#444] dark:text-[#a3a3a3] dark:hover:bg-[#2a2a2a]"
          :disabled="loading || skills.length === 0"
          @click="setAllEnabled(true)"
        >全部启用</Button>
        <Button
          variant="outline"
          size="sm"
          class="border-[#d1d5db] text-[#475569] hover:bg-[#f3f4f6] dark:border-[#444] dark:text-[#a3a3a3] dark:hover:bg-[#2a2a2a]"
          :disabled="loading || skills.length === 0"
          @click="setAllEnabled(false)"
        >全部停用</Button>
      </div>
    </div>

    <Card
      v-if="loading"
      class="border-[#e5e7eb] bg-[#f9fafb] dark:border-[#333] dark:bg-[#1e1e1e]"
    >
      <CardContent class="py-8 text-center text-[13.5px] text-[#64748b] dark:text-[#a3a3a3]">技能扫描中...</CardContent>
    </Card>

    <Card
      v-else-if="skills.length === 0"
      class="border-[#e5e7eb] bg-[#f9fafb] dark:border-[#333] dark:bg-[#1e1e1e]"
    >
      <CardContent class="py-8 text-center text-[13.5px] text-[#64748b] dark:text-[#a3a3a3]">
        未发现技能。请将技能放在应用数据目录的 skills 子目录（.../com.tauri-app.nova/skills/*/SKILL.md）。
      </CardContent>
    </Card>

    <div v-else class="flex flex-col gap-2">
      <Card
        v-for="skill in skills"
        :key="skill.path"
        class="gap-0 border-[#e5e7eb] py-3 dark:border-[#333]"
      >
        <CardHeader class="px-3 pb-1">
          <div class="flex min-w-0 items-center justify-between gap-3">
            <div class="min-w-0">
              <div class="flex items-center gap-2">
                <CardTitle class="truncate text-[13.5px] text-[#111827] dark:text-[#f3f4f6]">{{ skill.name }}</CardTitle>
                <span
                  class="shrink-0 rounded px-1.5 py-[1px] text-[10.5px] font-medium"
                  :class="skill.enabled ? 'bg-green-50 text-green-700 dark:bg-green-950/40 dark:text-green-400' : 'bg-[#f3f4f6] text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#9f9f9f]'"
                >{{ skill.enabled ? '已启用' : '已停用' }}</span>
              </div>
              <CardDescription class="mt-1 line-clamp-2 text-[12px] text-[#6b7280] dark:text-[#a3a3a3]">{{ skill.description }}</CardDescription>
              <div class="mt-1 truncate text-[11px] font-mono text-[#9ca3af] dark:text-[#666]" :title="skill.path">{{ skill.path }}</div>
            </div>
            <div class="flex shrink-0 items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-2.5 text-[12px] gap-1.5 border-[#d1d5db] text-[#334155] hover:bg-[#f3f4f6] dark:border-[#444] dark:text-[#cbd5e1] dark:hover:bg-[#2a2a2a]"
                @click="openViewSkill(skill)"
              >
                <FileText class="w-3.5 h-3.5 text-[#64748b] dark:text-[#94a3b8]" />
                查看
              </Button>
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-3 text-[12px]"
                @click="toggleSkill(skill)"
              >{{ skill.enabled ? '停用' : '启用' }}</Button>
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-2 text-[12px] border-red-200 text-red-600 hover:bg-red-50 dark:border-red-900/50 dark:text-red-400 dark:hover:bg-red-950/30"
                :disabled="deletingPath === skill.path"
                @click="requestDeleteSkill(skill)"
              >
                <Trash2 v-if="deletingPath !== skill.path" class="w-3.5 h-3.5" />
                <span v-else>...</span>
              </Button>
            </div>
          </div>
        </CardHeader>
      </Card>
    </div>

    <!-- 查看技能详情弹窗 -->
    <div
      v-if="viewingSkill"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm transition-all"
      @click.self="closeViewSkill"
    >
      <div
        class="flex flex-col rounded-2xl border border-[#e5e7eb] bg-white shadow-2xl transition-all duration-200 dark:border-[#27272a] dark:bg-[#18181b] overflow-hidden"
        :class="isMaximized ? 'w-[96vw] h-[93vh]' : 'w-full max-w-4xl h-[86vh]'"
      >
        <!-- Header -->
        <div class="flex shrink-0 items-center justify-between border-b border-[#e5e7eb] px-5 py-3.5 dark:border-[#27272a] bg-[#fafafa]/80 dark:bg-[#1c1c20]/80 backdrop-blur">
          <div class="flex items-center gap-3 min-w-0">
            <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-gradient-to-tr from-indigo-500 to-sky-400 text-white shadow-sm shadow-indigo-500/20">
              <Sparkles class="h-4.5 w-4.5" />
            </div>
            <div class="min-w-0">
              <div class="flex items-center gap-2.5">
                <h3 class="truncate text-[15px] font-semibold text-[#0f172a] dark:text-[#f8fafc]">
                  {{ viewingSkill.name }}
                </h3>
                <!-- 可点击就地切换开关的药丸徽章 -->
                <button
                  type="button"
                  class="group flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium transition-all cursor-pointer"
                  :class="viewingSkill.enabled 
                    ? 'bg-emerald-50 text-emerald-700 hover:bg-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-300 dark:hover:bg-emerald-900/50' 
                    : 'bg-[#f1f5f9] text-[#64748b] hover:bg-[#e2e8f0] dark:bg-[#27272a] dark:text-[#a1a1aa] dark:hover:bg-[#323238]'"
                  :title="viewingSkill.enabled ? '点击停用技能' : '点击启用技能'"
                  @click="toggleSkill(viewingSkill)"
                >
                  <span
                    class="h-1.5 w-1.5 rounded-full transition-all"
                    :class="viewingSkill.enabled ? 'bg-emerald-500 animate-pulse' : 'bg-[#94a3b8] dark:bg-[#71717a]'"
                  />
                  <span>{{ viewingSkill.enabled ? '已启用' : '已停用' }}</span>
                </button>
              </div>
            </div>
          </div>

          <div class="flex items-center gap-1">
            <button
              type="button"
              class="rounded-lg p-1.5 text-[#64748b] transition-colors hover:bg-[#f1f5f9] hover:text-[#0f172a] dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-[#f8fafc]"
              :title="isMaximized ? '还原窗口' : '最大化窗口'"
              @click="isMaximized = !isMaximized"
            >
              <Minimize2 v-if="isMaximized" class="h-4 w-4" />
              <Maximize2 v-else class="h-4 w-4" />
            </button>
            <button
              type="button"
              class="rounded-lg p-1.5 text-[#64748b] transition-colors hover:bg-[#f1f5f9] hover:text-[#0f172a] dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-[#f8fafc]"
              title="关闭"
              @click="closeViewSkill"
            >
              <X class="h-4 w-4" />
            </button>
          </div>
        </div>

        <!-- 面包屑与路径状态栏 (VS Code 风格) -->
        <div class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-[#e5e7eb] bg-[#f8fafc] px-5 py-2 text-[11.5px] dark:border-[#27272a] dark:bg-[#17171a]">
          <div class="flex items-center gap-2 min-w-0 max-w-[70%]">
            <span class="truncate font-mono text-[#64748b] dark:text-[#94a3b8]" :title="viewingSkill.path">
              {{ viewingSkill.path }}
            </span>
            <button
              type="button"
              class="shrink-0 flex items-center gap-1 text-[11px] text-[#2563eb] hover:text-[#1d4ed8] dark:text-[#60a5fa] dark:hover:text-[#93c5fd] transition-colors"
              title="复制完整路径"
              @click="copySkillPath"
            >
              <Check v-if="pathCopied" class="h-3 w-3 text-emerald-600 dark:text-emerald-400" />
              <Copy v-else class="h-3 w-3" />
              <span>{{ pathCopied ? '已复制' : '复制路径' }}</span>
            </button>
            <span class="text-[#cbd5e1] dark:text-[#3f3f46]">|</span>
            <button
              type="button"
              class="shrink-0 flex items-center gap-1 text-[11px] text-[#2563eb] hover:text-[#1d4ed8] dark:text-[#60a5fa] dark:hover:text-[#93c5fd] transition-colors"
              title="在系统文件管理器中定位"
              @click="openInExplorer"
            >
              <FolderOpen class="h-3.5 w-3.5" />
              <span>打开目录</span>
            </button>
          </div>

          <div class="flex items-center gap-3 text-[#94a3b8] dark:text-[#71717a] font-mono text-[11px]">
            <span>{{ parsedSkillDoc.lineCount }} 行</span>
            <span>·</span>
            <span>{{ formatFileSize(parsedSkillDoc.byteSize) }}</span>
          </div>
        </div>

        <!-- Toolbar: 模式切换 & 快捷操作 -->
        <div class="flex shrink-0 items-center justify-between border-b border-[#e5e7eb] px-5 py-2.5 dark:border-[#27272a] bg-white dark:bg-[#18181b]">
          <!-- 药丸风格 Tab 切换 -->
          <div class="inline-flex items-center rounded-lg bg-[#f1f5f9] p-0.5 dark:bg-[#27272a]">
            <button
              type="button"
              class="flex items-center gap-1.5 rounded-md px-3 py-1 text-[12px] font-medium transition-all cursor-pointer"
              :class="viewingMode === 'preview'
                ? 'bg-white text-[#0f172a] shadow-sm dark:bg-[#18181b] dark:text-[#f8fafc]'
                : 'text-[#64748b] hover:text-[#0f172a] dark:text-[#a1a1aa] dark:hover:text-[#f8fafc]'"
              @click="viewingMode = 'preview'"
            >
              <BookOpen class="h-3.5 w-3.5" />
              <span>文档预览</span>
            </button>
            <button
              type="button"
              class="flex items-center gap-1.5 rounded-md px-3 py-1 text-[12px] font-medium transition-all cursor-pointer"
              :class="viewingMode === 'source'
                ? 'bg-white text-[#0f172a] shadow-sm dark:bg-[#18181b] dark:text-[#f8fafc]'
                : 'text-[#64748b] hover:text-[#0f172a] dark:text-[#a1a1aa] dark:hover:text-[#f8fafc]'"
              @click="viewingMode = 'source'"
            >
              <Code2 class="h-3.5 w-3.5" />
              <span>原始代码</span>
            </button>
          </div>

          <div class="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              class="h-7.5 px-3 text-[12px] gap-1.5 border-[#e2e8f0] dark:border-[#27272a]"
              :disabled="viewingLoading || !viewingContent"
              @click="copySkillContent"
            >
              <Check v-if="copied" class="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
              <Copy v-else class="h-3.5 w-3.5 text-[#64748b] dark:text-[#a1a1aa]" />
              <span>{{ copied ? '已复制全文' : '复制全文' }}</span>
            </Button>
          </div>
        </div>

        <!-- 内容区域 -->
        <div class="flex-1 overflow-y-auto px-6 py-5 custom-scrollbar min-h-0 bg-[#ffffff] dark:bg-[#18181b]">
          <!-- 加载中 -->
          <div v-if="viewingLoading" class="flex flex-col items-center justify-center py-20 text-[#64748b] dark:text-[#a1a1aa]">
            <RotateCw class="h-6 w-6 animate-spin text-[#3b82f6] mb-3" />
            <span class="text-[13px]">加载技能内容中...</span>
          </div>

          <!-- 报错 -->
          <div v-else-if="viewingError" class="rounded-xl border border-red-200 bg-red-50/50 p-6 text-center text-[13px] text-red-600 dark:border-red-900/40 dark:bg-red-950/20 dark:text-red-400">
            {{ viewingError }}
          </div>

          <!-- 空白内容 -->
          <div v-else-if="!viewingContent" class="py-20 text-center text-[13px] text-[#94a3b8] dark:text-[#71717a]">
            技能文件为空
          </div>

          <!-- 预览模式 (Markdown 优雅呈现) -->
          <div v-else-if="viewingMode === 'preview'" class="mx-auto max-w-4xl space-y-6">
            <!-- 结构化 Frontmatter 元数据展示卡片 -->
            <div
              v-if="parsedSkillDoc.hasFrontmatter"
              class="rounded-xl border border-[#e2e8f0] bg-gradient-to-b from-[#f8fafc] to-[#f1f5f9]/50 p-4 shadow-xs dark:border-[#27272a] dark:from-[#202024] dark:to-[#18181b]"
            >
              <div class="flex items-start justify-between gap-4">
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2">
                    <span class="rounded bg-[#e0e7ff] px-2 py-0.5 text-[11px] font-semibold text-[#4338ca] dark:bg-[#312e81]/60 dark:text-[#a5b4fc]">
                      SKILL DEFINITION
                    </span>
                    <span class="font-mono text-[13px] font-semibold text-[#0f172a] dark:text-[#f8fafc]">
                      {{ parsedSkillDoc.frontmatter['name'] || viewingSkill.name }}
                    </span>
                  </div>

                  <p v-if="parsedSkillDoc.frontmatter['description']" class="mt-2 text-[12.5px] leading-relaxed text-[#475569] dark:text-[#cbd5e1]">
                    {{ parsedSkillDoc.frontmatter['description'] }}
                  </p>

                  <!-- 其它元数据属性 Tags (除了 name 和 description 之外的键) -->
                  <div class="mt-3 flex flex-wrap items-center gap-2 pt-2 border-t border-[#e2e8f0]/60 dark:border-[#27272a]/60">
                    <div
                      v-for="entry in parsedSkillDoc.frontmatterEntries.filter(e => e.key !== 'name' && e.key !== 'description')"
                      :key="entry.key"
                      class="flex items-center rounded-md border border-[#e2e8f0] bg-white px-2 py-1 text-[11px] shadow-2xs dark:border-[#333] dark:bg-[#18181b]"
                    >
                      <span class="font-semibold text-[#64748b] dark:text-[#94a3b8] mr-1.5">{{ entry.key }}:</span>
                      <span class="font-mono text-[#0f172a] dark:text-[#e4e4e7]">{{ entry.value }}</span>
                    </div>

                    <!-- 展开原始 YAML 切换 -->
                    <button
                      type="button"
                      class="flex items-center gap-1 text-[11px] text-[#64748b] hover:text-[#0f172a] dark:text-[#a1a1aa] dark:hover:text-[#f8fafc] ml-auto transition-colors cursor-pointer"
                      @click="showRawFrontmatter = !showRawFrontmatter"
                    >
                      <component :is="showRawFrontmatter ? ChevronDown : ChevronRight" class="h-3 w-3" />
                      <span>{{ showRawFrontmatter ? '收起配置元数据' : '查看 YAML 原始配置' }}</span>
                    </button>
                  </div>

                  <!-- 折叠展示的原始 YAML -->
                  <pre
                    v-if="showRawFrontmatter"
                    class="mt-3 rounded-lg border border-[#e2e8f0] bg-white p-3 font-mono text-[11px] leading-5 text-[#334155] dark:border-[#27272a] dark:bg-[#121214] dark:text-[#cbd5e1] overflow-x-auto"
                  >{{ parsedSkillDoc.rawFrontmatter }}</pre>
                </div>
              </div>
            </div>

            <!-- Markdown 正文 (MarkdownRenderer 带有全套优雅样式和代码高亮/复制) -->
            <div class="rounded-xl border border-transparent pt-1">
              <MarkdownRenderer
                v-if="parsedSkillDoc.body"
                :content="parsedSkillDoc.body"
              />
              <div v-else class="py-8 text-center text-[12.5px] text-[#94a3b8] dark:text-[#71717a]">
                该技能无正文文档，仅包含元数据配置
              </div>
            </div>
          </div>

          <!-- 源码模式 (带行号的等宽高质感代码查看器) -->
          <div v-else class="mx-auto max-w-4xl">
            <div class="relative flex overflow-x-auto rounded-xl border border-[#e2e8f0] bg-[#f8fafc] font-mono text-[12.5px] leading-6 shadow-xs dark:border-[#27272a] dark:bg-[#121214]">
              <!-- 行号栏 -->
              <div class="shrink-0 select-none border-r border-[#e2e8f0] bg-[#f1f5f9]/70 py-3.5 pl-3.5 pr-3 text-right text-[11.5px] text-[#94a3b8] dark:border-[#27272a] dark:bg-[#18181b]/70 dark:text-[#52525b]">
                <div v-for="(_, index) in sourceLines" :key="index" class="h-6 leading-6">
                  {{ index + 1 }}
                </div>
              </div>
              <!-- 源码行内容 -->
              <pre class="flex-1 overflow-x-auto py-3.5 px-4.5 text-[#1e293b] dark:text-[#e4e4e7] whitespace-pre select-text"><div v-for="(line, index) in sourceLines" :key="index" class="h-6 leading-6">{{ line || ' ' }}</div></pre>
            </div>
          </div>
        </div>

        <!-- Footer -->
        <div class="flex shrink-0 items-center justify-between border-t border-[#e5e7eb] px-6 py-3.5 dark:border-[#27272a] bg-[#fafafa]/80 dark:bg-[#1c1c20]/80">
          <div class="flex items-center gap-2 text-[12px] text-[#64748b] dark:text-[#a1a1aa]">
            <span>状态：</span>
            <span
              class="font-medium"
              :class="viewingSkill.enabled ? 'text-emerald-600 dark:text-emerald-400' : 'text-[#64748b] dark:text-[#71717a]'"
            >
              {{ viewingSkill.enabled ? '已激活供 Agent 识别调用' : '已停用' }}
            </span>
          </div>

          <div class="flex items-center gap-2.5">
            <Button
              variant="outline"
              size="sm"
              class="px-3 text-[12px]"
              @click="toggleSkill(viewingSkill)"
            >
              {{ viewingSkill.enabled ? '停用此技能' : '启用此技能' }}
            </Button>
            <Button
              size="sm"
              class="px-4 text-[12px] bg-[#0f172a] text-white hover:bg-[#1e293b] dark:bg-[#f8fafc] dark:text-[#0f172a] dark:hover:bg-[#e2e8f0]"
              @click="closeViewSkill"
            >
              完成
            </Button>
          </div>
        </div>
      </div>
    </div>

    <div class="mt-auto border-t border-[#e5e7eb] pt-4 dark:border-[#333]">
      <div v-if="error" class="mb-2 text-[12.5px] text-red-600 dark:text-red-400">{{ error }}</div>
      <div v-if="deleteError" class="mb-2 text-[12.5px] text-red-600 dark:text-red-400">{{ deleteError }}</div>
      <div class="flex items-center justify-end">
        <span v-if="saving" class="text-[13px] text-[#64748b] dark:text-[#a3a3a3]">保存中...</span>
        <span v-else-if="savedTip" class="text-[13px] text-[#4f9c64] dark:text-[#62c07a]">✓ 已保存</span>
      </div>
    </div>
  </div>
</template>

