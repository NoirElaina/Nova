<script setup lang="ts">
import { ref, watch } from 'vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

export interface ProviderDraft {
  id: string
  displayName: string
  apiFormat: string
  apiKey: string
  baseUrl: string
  model: string
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

const localDraft = ref<ProviderDraft>({
  id: '',
  displayName: '',
  apiFormat: 'openai',
  apiKey: '',
  baseUrl: '',
  model: ''
})

watch(() => props.open, (newVal) => {
  if (newVal && props.draft) {
    localDraft.value = { ...props.draft }
  } else if (newVal && !props.draft) {
    localDraft.value = {
      id: '',
      displayName: '',
      apiFormat: 'openai',
      apiKey: '',
      baseUrl: '',
      model: ''
    }
  }
})

const handleSave = () => {
  if (!localDraft.value.id.trim()) {
    localDraft.value.id = localDraft.value.displayName.toLowerCase().replace(/[^a-z0-9]/g, '-')
  }
  emit('save', localDraft.value, props.draft?.id || null)
  emit('update:open', false)
}
</script>

<template>
  <div v-if="open" class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-sm sm:p-0">
    <div class="w-full max-w-md rounded-xl border bg-background p-6 shadow-lg sm:p-8">
      <div class="mb-6">
        <h2 class="text-xl font-bold tracking-tight">{{ isNew ? '添加模型配置' : '编辑模型配置' }}</h2>
      </div>

      <div class="grid gap-4 py-2">
        <div class="grid gap-2">
          <Label for="displayName">显示名称</Label>
          <Input id="displayName" v-model="localDraft.displayName" placeholder="例如: OpenAI" />
        </div>

        <div class="grid gap-2">
          <Label for="id">内部标识符 (ID)</Label>
          <Input id="id" v-model="localDraft.id" placeholder="唯一标识符,留空自动生成" :disabled="!isNew" />
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
          <Label for="model">默认模型</Label>
          <Input id="model" v-model="localDraft.model" placeholder="例如: gpt-4o" />
        </div>
      </div>

      <div class="mt-6 flex justify-end gap-3">
        <Button variant="outline" @click="emit('update:open', false)">取消</Button>
        <Button @click="handleSave">保存</Button>
      </div>
    </div>
  </div>
</template>
