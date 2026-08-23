<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import {
  applyUiTheme,
  getStoredUiLanguage,
  getStoredUiTheme,
  normalizeUiLanguage,
  normalizeUiTheme,
  setStoredUiLanguage,
  setStoredUiTheme,
  type UiLanguage,
  type UiTheme,
} from '../../../../lib/ui-preferences'

const theme = ref<UiTheme>(getStoredUiTheme())
const language = ref<UiLanguage>(getStoredUiLanguage())
const enableAppLog = ref(false)
const approvalPolicy = ref<'always_ask' | 'on_request' | 'never'>('on_request')
const progressiveToolDisclosure = ref(true)
const permissionRules = ref<{ kind: string; signature: string; createdAtMs: number }[]>([])
const isSavingPreferences = ref(false)
const cachedSettings = ref<Record<string, unknown> | null>(null)

const localeTexts = {
  'zh-CN': {
    appearanceTitle: '外观',
    appearanceDesc: '选择 Nova 在你的设备上的显示方式。',
    themeLabel: '主题',
    languageTitle: '语言',
    languageDesc: '切换界面显示语言。',
    loggingTitle: '软件日志',
    loggingDesc: '控制是否将统一软件日志写入本地日志文件。',
    loggingSwitchLabel: '记录软件日志到本地文件',
    securityTitle: '安全与自动化',
    securityDesc: '控制工具执行审批与工具加载策略。',
    approvalPolicyLabel: '审批策略',
    approvalPolicyAlwaysAsk: '每次都问（严格）',
    approvalPolicyOnRequest: '仅风险操作询问（推荐）',
    approvalPolicyNever: '从不询问（完全自动）',
    disclosureLabel: '渐进式工具披露',
    disclosureDesc: '低频工具不默认进入模型工具清单，由 AI 通过 LoadTool 按需加载，节省上下文。',
    rulesTitle: '已记住的权限规则',
    rulesEmpty: '暂无持久化规则。审批时选择“始终允许”后会记录在这里。',
    rulesDelete: '删除',
    settingsSaveFailed: '保存设置失败：',
    themeSystem: '系统',
    themeLight: '浅色',
    themeDark: '深色',
    languageEnglish: 'English',
    languageChinese: '简体中文',
  },
  'en-US': {
    appearanceTitle: 'Appearance',
    appearanceDesc: 'Select how Nova looks on your device.',
    themeLabel: 'Theme',
    languageTitle: 'Language',
    languageDesc: 'Change the interface language.',
    loggingTitle: 'Application Logging',
    loggingDesc: 'Control whether the unified application log is written to local log files.',
    loggingSwitchLabel: 'Write application logs to local files',
    securityTitle: 'Security & Automation',
    securityDesc: 'Control tool approval and tool loading behavior.',
    approvalPolicyLabel: 'Approval policy',
    approvalPolicyAlwaysAsk: 'Always ask (strict)',
    approvalPolicyOnRequest: 'Ask for risky only (recommended)',
    approvalPolicyNever: 'Never ask (fully automatic)',
    disclosureLabel: 'Progressive tool disclosure',
    disclosureDesc: 'Low-frequency tools stay out of the default tool list; the AI loads them on demand via LoadTool, saving context.',
    rulesTitle: 'Remembered permission rules',
    rulesEmpty: 'No persisted rules yet. Choosing “Always allow” during approval records a rule here.',
    rulesDelete: 'Delete',
    settingsSaveFailed: 'Failed to save settings: ',
    themeSystem: 'System',
    themeLight: 'Light',
    themeDark: 'Dark',
    languageEnglish: 'English',
    languageChinese: '简体中文',
  },
} as const

const t = computed(() => localeTexts[language.value])

const themeOptions = computed(() => [
  { value: 'system' as UiTheme, label: t.value.themeSystem },
  { value: 'light' as UiTheme, label: t.value.themeLight },
  { value: 'dark' as UiTheme, label: t.value.themeDark },
])

const dispatchLanguageUpdated = () => {
  window.dispatchEvent(
    new CustomEvent('ui-language-updated', {
      detail: { language: language.value },
    }),
  )
}

const loadSettings = async () => {
  try {
    const settings = await invoke<Record<string, unknown>>('get_settings')
    cachedSettings.value = settings

    const nextLanguage = normalizeUiLanguage(settings.uiLanguage)
    const nextTheme = normalizeUiTheme(settings.uiTheme)
    const nextEnableAppLog = settings.enableAppLog === true
    language.value = nextLanguage
    theme.value = nextTheme
    enableAppLog.value = nextEnableAppLog

    const policy = settings.approvalPolicy
    approvalPolicy.value =
      policy === 'always_ask' || policy === 'never' ? policy : 'on_request'
    progressiveToolDisclosure.value = settings.progressiveToolDisclosure !== false

    setStoredUiLanguage(nextLanguage)
    setStoredUiTheme(nextTheme)
    applyUiTheme(nextTheme)
    dispatchLanguageUpdated()
  } catch (error) {
    console.error('Failed to load general settings:', error)
    applyUiTheme(theme.value)
    dispatchLanguageUpdated()
  }
}

const loadPermissionRules = async () => {
  try {
    permissionRules.value = await invoke('list_permission_rules')
  } catch (error) {
    console.error('Failed to load permission rules:', error)
    permissionRules.value = []
  }
}

const deletePermissionRule = async (signature: string) => {
  try {
    await invoke('delete_permission_rule', { signature })
    await loadPermissionRules()
  } catch (error) {
    console.error('Failed to delete permission rule:', error)
  }
}

const persistPreferences = async () => {
  if (isSavingPreferences.value) {
    return
  }

  isSavingPreferences.value = true
  try {
    // 始终取最新设置作为基底：挂载时缓存的副本可能已过期，
    // 用旧副本展开保存会把其他字段回滚。
    const baseSettings = await invoke<Record<string, unknown>>('get_settings')
    const nextSettings: Record<string, unknown> = {
      ...baseSettings,
      uiLanguage: language.value,
      uiTheme: theme.value,
      enableAppLog: enableAppLog.value,
      approvalPolicy: approvalPolicy.value,
      progressiveToolDisclosure: progressiveToolDisclosure.value,
    }

    cachedSettings.value = nextSettings
    await invoke('save_settings', { settings: nextSettings })
    window.dispatchEvent(new CustomEvent('settings-updated'))
  } catch (error) {
    console.error('Failed to save general settings:', error)
  } finally {
    isSavingPreferences.value = false
  }
}

const setTheme = (value: UiTheme) => {
  const normalized = normalizeUiTheme(value)
  theme.value = normalized
  setStoredUiTheme(normalized)
  applyUiTheme(normalized)
  void persistPreferences()
}

const onLanguageChange = () => {
  const normalized = normalizeUiLanguage(language.value)
  language.value = normalized
  setStoredUiLanguage(normalized)
  dispatchLanguageUpdated()
  void persistPreferences()
}

const onLanguageSelect = (value: string) => {
  language.value = normalizeUiLanguage(value)
  onLanguageChange()
}

const onEnableAppLogChange = (checked: boolean | 'indeterminate') => {
  enableAppLog.value = checked === true
  void persistPreferences()
}

const onApprovalPolicySelect = (value: string) => {
  approvalPolicy.value =
    value === 'always_ask' || value === 'never' ? value : 'on_request'
  void persistPreferences()
}

const onDisclosureChange = (checked: boolean | 'indeterminate') => {
  progressiveToolDisclosure.value = checked === true
  void persistPreferences()
}

onMounted(() => {
  void loadSettings()
  void loadPermissionRules()
})
</script>

<template>
  <div class="flex max-w-3xl flex-col gap-4">
    <!-- 外观：主题 + 语言合并为一张卡片，每行左标题右控件 -->
    <Card class="rounded-xl border-[#e7e9ee] dark:border-[#343434]">
      <CardHeader class="pb-1">
        <CardTitle class="text-[0.9rem]">{{ t.appearanceTitle }}</CardTitle>
        <CardDescription>{{ t.appearanceDesc }}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col">
        <!-- 主题：分段选择器 -->
        <div class="flex items-center justify-between gap-4 py-2.5">
          <span class="text-[0.9rem] text-[#374151] dark:text-[#d7d7d7]">{{ t.themeLabel }}</span>
          <div class="flex items-center gap-0.5 rounded-lg bg-[#f3f4f6] p-0.5 dark:bg-white/5">
            <button
              v-for="opt in themeOptions"
              :key="opt.value"
              type="button"
              class="h-7 rounded-md px-3 text-[12.5px] transition-colors"
              :class="theme === opt.value
                ? 'bg-white font-medium text-[#111827] shadow-sm ring-1 ring-black/5 dark:bg-[#3a3a3a] dark:text-[#ececec] dark:ring-white/10'
                : 'text-[#64748b] hover:text-[#111827] dark:text-[#9ca3af] dark:hover:text-[#ececec]'"
              @click="setTheme(opt.value)"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>
        <!-- 语言 -->
        <div class="flex items-center justify-between gap-4 border-t border-[#eef0f3] py-2.5 dark:border-[#2c2c2c]">
          <div class="min-w-0">
            <div class="text-[0.9rem] text-[#374151] dark:text-[#d7d7d7]">{{ t.languageTitle }}</div>
            <p class="mt-0.5 text-xs text-[#7b8494] dark:text-[#9ca3af]">{{ t.languageDesc }}</p>
          </div>
          <Select :model-value="language" @update:model-value="(value) => onLanguageSelect(String(value))">
            <SelectTrigger class="h-8 w-[160px] shrink-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="zh-CN">{{ t.languageChinese }}</SelectItem>
              <SelectItem value="en-US">{{ t.languageEnglish }}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </CardContent>
    </Card>

    <!-- 安全与自动化 -->
    <Card class="rounded-xl border-[#e7e9ee] dark:border-[#343434]">
      <CardHeader class="pb-1">
        <CardTitle class="text-[0.9rem]">{{ t.securityTitle }}</CardTitle>
        <CardDescription>{{ t.securityDesc }}</CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col">
        <!-- 审批策略 -->
        <div class="flex items-center justify-between gap-4 py-2.5">
          <span class="text-[0.9rem] text-[#374151] dark:text-[#d7d7d7]">{{ t.approvalPolicyLabel }}</span>
          <Select :model-value="approvalPolicy" @update:model-value="(value) => onApprovalPolicySelect(String(value))">
            <SelectTrigger class="h-8 w-[240px] shrink-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="always_ask">{{ t.approvalPolicyAlwaysAsk }}</SelectItem>
              <SelectItem value="on_request">{{ t.approvalPolicyOnRequest }}</SelectItem>
              <SelectItem value="never">{{ t.approvalPolicyNever }}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <!-- 渐进式工具披露 -->
        <div class="flex items-center justify-between gap-4 border-t border-[#eef0f3] py-2.5 dark:border-[#2c2c2c]">
          <div class="min-w-0">
            <Label for="general-progressive-disclosure" class="text-[0.9rem] font-normal text-[#374151] dark:text-[#d7d7d7]">
              {{ t.disclosureLabel }}
            </Label>
            <p class="mt-0.5 text-xs text-[#7b8494] dark:text-[#9ca3af]">{{ t.disclosureDesc }}</p>
          </div>
          <Checkbox
            id="general-progressive-disclosure"
            class="shrink-0"
            :model-value="progressiveToolDisclosure"
            @update:model-value="onDisclosureChange"
          />
        </div>
        <!-- 已记住的权限规则 -->
        <div class="space-y-1.5 border-t border-[#eef0f3] py-2.5 dark:border-[#2c2c2c]">
          <Label class="text-[0.9rem] font-normal text-[#374151] dark:text-[#d7d7d7]">
            {{ t.rulesTitle }}
          </Label>
          <p v-if="permissionRules.length === 0" class="text-xs text-[#7b8494] dark:text-[#9ca3af]">
            {{ t.rulesEmpty }}
          </p>
          <ul v-else class="space-y-1">
            <li
              v-for="rule in permissionRules"
              :key="`${rule.kind}:${rule.signature}`"
              class="flex items-center justify-between gap-2 rounded-md border border-[#e5e7eb] px-2.5 py-1.5 dark:border-[#333]"
            >
              <div class="min-w-0">
                <span
                  class="mr-2 inline-block rounded px-1.5 py-0.5 text-[10px] font-medium"
                  :class="rule.kind === 'allow'
                    ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300'
                    : 'bg-red-50 text-red-700 dark:bg-red-950 dark:text-red-300'"
                >
                  {{ rule.kind === 'allow' ? 'ALLOW' : 'DENY' }}
                </span>
                <span class="break-all font-mono text-xs text-[#374151] dark:text-[#d7d7d7]">{{ rule.signature }}</span>
              </div>
              <Button size="sm" variant="outline" class="h-6 shrink-0 px-2 text-xs" @click="deletePermissionRule(rule.signature)">
                {{ t.rulesDelete }}
              </Button>
            </li>
          </ul>
        </div>
      </CardContent>
    </Card>

    <!-- 软件日志 -->
    <Card class="rounded-xl border-[#e7e9ee] dark:border-[#343434]">
      <CardContent class="flex items-center justify-between gap-4 py-3">
        <div class="min-w-0">
          <Label for="general-enable-app-log" class="text-[0.9rem] font-normal text-[#374151] dark:text-[#d7d7d7]">
            {{ t.loggingSwitchLabel }}
          </Label>
          <p class="mt-0.5 text-xs text-[#7b8494] dark:text-[#9ca3af]">{{ t.loggingDesc }}</p>
        </div>
        <Checkbox
          id="general-enable-app-log"
          class="shrink-0"
          :model-value="enableAppLog"
          @update:model-value="onEnableAppLogChange"
        />
      </CardContent>
    </Card>
  </div>
</template>
