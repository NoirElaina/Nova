<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

const deletingPath = ref<string | null>(null)
const deleteError = ref('')

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
}

const save = async () => {
  saving.value = true
  error.value = ''
  try {
    const listed = new Set(skills.value.map((s) => normalize(s.name)))
    const existingDisabled = (Array.isArray(rawSettings.value?.disabledSkills)
      ? rawSettings.value.disabledSkills
      : [])
      .filter((v: unknown) => typeof v === 'string')

    const preservedDisabled = existingDisabled.filter((name: string) => !listed.has(normalize(name)))
    const currentDisabled = skills.value.filter((s) => !s.enabled).map((s) => s.name)

    const settings = {
      ...rawSettings.value,
      disabledSkills: [...preservedDisabled, ...currentDisabled],
    }

    await invoke('save_settings', { settings })
    rawSettings.value = settings
    savedTip.value = true
    setTimeout(() => (savedTip.value = false), 2000)
  } catch (e) {
    console.error('Failed to save skill settings:', e)
  } finally {
    saving.value = false
  }
}

onMounted(refresh)

const deleteSkill = async (skill: SkillItem) => {
  if (!confirm(`确定要删除技能「${skill.name}」吗？此操作将永久删除该技能目录，无法恢复。`)) return
  deletingPath.value = skill.path
  deleteError.value = ''
  try {
    await invoke('delete_skill', { path: skill.path })
    await refresh()
  } catch (e) {
    console.error(`Failed to delete skill (${skill.path}):`, e)
  } finally {
    deletingPath.value = null
  }
}
</script>

<template>
  <div class="px-6 py-4 flex flex-col h-full overflow-y-auto">
    <div class="mb-4 flex items-center justify-between">
      <span class="text-[12.5px] text-[#64748b] dark:text-[#a3a3a3]">{{ skills.length }} 个技能</span>
      <div class="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          class="border-[#d1d5db] text-[#475569] hover:bg-[#f3f4f6] dark:border-[#444] dark:text-[#a3a3a3] dark:hover:bg-[#2a2a2a]"
          :disabled="loading"
          @click="refresh"
        >刷新</Button>
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
              <CardTitle class="truncate text-[13.5px] text-[#111827] dark:text-[#f3f4f6]">{{ skill.name }}</CardTitle>
              <CardDescription class="mt-1 line-clamp-2 text-[12px] text-[#6b7280] dark:text-[#a3a3a3]">{{ skill.description }}</CardDescription>
              <div class="mt-1 truncate text-[11px] text-[#9ca3af] dark:text-[#666]" :title="skill.path">{{ skill.path }}</div>
            </div>
            <div class="flex shrink-0 items-center gap-2">
              <span
                class="rounded px-1.5 py-[1px] text-[11px]"
                :class="skill.enabled ? 'bg-green-50 text-green-700 dark:bg-green-950/30 dark:text-green-400' : 'bg-[#f3f4f6] text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#9f9f9f]'"
              >{{ skill.enabled ? '已启用' : '已停用' }}</span>
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-3 text-[12px]"
                @click="skill.enabled = !skill.enabled"
              >{{ skill.enabled ? '停用' : '启用' }}</Button>
              <Button
                variant="outline"
                size="sm"
                class="h-7 px-2 text-[12px] border-red-200 text-red-600 hover:bg-red-50 dark:border-red-900/50 dark:text-red-400 dark:hover:bg-red-950/30"
                :disabled="deletingPath === skill.path"
                @click="deleteSkill(skill)"
              >
                <svg v-if="deletingPath !== skill.path" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg>
                <span v-else>...</span>
              </Button>
            </div>
          </div>
        </CardHeader>
      </Card>
    </div>

    <div class="mt-auto border-t border-[#e5e7eb] pt-4 dark:border-[#333]">
      <div v-if="error" class="mb-2 text-[12.5px] text-red-600 dark:text-red-400">{{ error }}</div>
      <div v-if="deleteError" class="mb-2 text-[12.5px] text-red-600 dark:text-red-400">{{ deleteError }}</div>
      <div class="flex items-center justify-end gap-3">
        <span v-if="savedTip" class="text-[13px] text-[#4f9c64] dark:text-[#62c07a]">✓ 已保存</span>
        <Button
          size="sm"
          class="bg-primary text-primary-foreground hover:bg-primary/90"
          :disabled="saving"
          @click="save"
        >{{ saving ? '保存中...' : '保存设置' }}</Button>
      </div>
    </div>
  </div>
</template>
