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
const isSavingPreferences = ref(false)
const cachedSettings = ref<Record<string, unknown> | null>(null)

const localeTexts = {
  'zh-CN': {
    appearanceTitle: '外观',
    appearanceDesc: '选择 Nova 在你的设备上的显示方式。',
    languageTitle: '语言',
    languageDesc: '切换界面显示语言。',
    loggingTitle: '软件日志',
    loggingDesc: '控制是否将统一软件日志写入本地日志文件。',
    loggingSwitchLabel: '记录软件日志到本地文件',
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
    languageTitle: 'Language',
    languageDesc: 'Change the interface language.',
    loggingTitle: 'Application Logging',
    loggingDesc: 'Control whether the unified application log is written to local log files.',
    loggingSwitchLabel: 'Write application logs to local files',
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

const persistPreferences = async () => {
  if (isSavingPreferences.value) {
    return
  }

  isSavingPreferences.value = true
  try {
    const baseSettings = cachedSettings.value ?? await invoke<Record<string, unknown>>('get_settings')
    const nextSettings: Record<string, unknown> = {
      ...baseSettings,
      uiLanguage: language.value,
      uiTheme: theme.value,
      enableAppLog: enableAppLog.value,
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

onMounted(() => {
  void loadSettings()
})
</script>

<template>
  <div class="flex flex-col gap-3">
    <Card class="border-[#e5e7eb] dark:border-[#333]">
      <CardHeader class="pb-2">
        <CardTitle class="text-[0.9rem]">{{ t.appearanceTitle }}</CardTitle>
        <CardDescription>{{ t.appearanceDesc }}</CardDescription>
      </CardHeader>
      <CardContent>
        <div class="flex flex-wrap gap-2">
          <Button
            v-for="opt in themeOptions"
            :key="opt.value"
            size="sm"
            :variant="theme === opt.value ? 'default' : 'outline'"
            class="min-w-[88px]"
            @click="setTheme(opt.value)"
          >
            {{ opt.label }}
          </Button>
        </div>
      </CardContent>
    </Card>

    <Card class="border-[#e5e7eb] dark:border-[#333]">
      <CardHeader class="pb-2">
        <CardTitle class="text-[0.9rem]">{{ t.languageTitle }}</CardTitle>
        <CardDescription>{{ t.languageDesc }}</CardDescription>
      </CardHeader>
      <CardContent>
        <Select :model-value="language" @update:model-value="(value) => onLanguageSelect(String(value))">
          <SelectTrigger class="w-[180px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="zh-CN">{{ t.languageChinese }}</SelectItem>
            <SelectItem value="en-US">{{ t.languageEnglish }}</SelectItem>
          </SelectContent>
        </Select>
      </CardContent>
    </Card>

    <Card class="border-[#e5e7eb] dark:border-[#333]">
      <CardHeader class="pb-2">
        <CardTitle class="text-[0.9rem]">{{ t.loggingTitle }}</CardTitle>
        <CardDescription>{{ t.loggingDesc }}</CardDescription>
      </CardHeader>
      <CardContent>
        <div class="flex items-center gap-3">
          <Checkbox
            id="general-enable-app-log"
            :model-value="enableAppLog"
            @update:model-value="onEnableAppLogChange"
          />
          <Label for="general-enable-app-log" class="text-[0.9rem] font-normal text-[#374151] dark:text-[#d7d7d7]">
            {{ t.loggingSwitchLabel }}
          </Label>
        </div>
      </CardContent>
    </Card>
  </div>
</template>
