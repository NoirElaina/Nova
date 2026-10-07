<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Button } from '@/components/ui/button'

import GeneralTab from './tabs/GeneralTab.vue'
import ModelTab   from './tabs/ModelTab.vue'
import McpTab     from './tabs/McpTab.vue'
import RagTab     from './tabs/RagTab.vue'
import SkillTab   from './tabs/SkillTab.vue'
import DataTab    from './tabs/DataTab.vue'
import MemoryTab  from './tabs/MemoryTab.vue'
import AboutTab   from './tabs/AboutTab.vue'
import {
  getStoredUiLanguage,
  normalizeUiLanguage,
  type UiLanguage,
} from '@/lib/ui-preferences'

const props = defineProps<{ modelValue: boolean }>()
const emit  = defineEmits<{ 'update:modelValue': [val: boolean] }>()
const close = () => emit('update:modelValue', false)

type Tab = 'general' | 'model' | 'mcp' | 'rag' | 'skill' | 'memory' | 'data' | 'about'
const activeTab = ref<Tab>('general')
const uiLanguage = ref<UiLanguage>(getStoredUiLanguage())

const tabs: { id: Tab; icon: string }[] = [
  { id: 'general', icon: 'M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z' },
  { id: 'model', icon: 'M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z' },
  { id: 'mcp', icon: 'M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z' },
  { id: 'rag', icon: 'M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.746 0 3.332.477 4.5 1.253v13C19.832 18.477 18.246 18 16.5 18c-1.746 0-3.332.477-4.5 1.253' },
  { id: 'skill', icon: 'M9.813 15.904A3 3 0 1012.087 18M5.143 4.567a3 3 0 103.707 3.707M18.36 5.143a3 3 0 10-3.707 3.707' },
  { id: 'memory', icon: 'M9 12h6M9 16h6M9 8h6M6 3h12a2 2 0 012 2v14a2 2 0 01-2 2H6a2 2 0 01-2-2V5a2 2 0 012-2z' },
  { id: 'data', icon: 'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4' },
  { id: 'about', icon: 'M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z' },
]

const tabLabelByLanguage: Record<UiLanguage, Record<Tab, string>> = {
  'zh-CN': {
    general: '通用',
    model: '模型',
    mcp: 'MCP',
    rag: 'RAG',
    skill: '技能',
    memory: '记忆',
    data: '数据',
    about: '关于',
  },
  'en-US': {
    general: 'General',
    model: 'Models',
    mcp: 'MCP',
    rag: 'RAG',
    skill: 'Skills',
    memory: 'Memory',
    data: 'Data',
    about: 'About',
  },
}

const closeTitle = computed(() => (uiLanguage.value === 'zh-CN' ? '关闭' : 'Close'))
const currentTabTitle = computed(() => tabLabelByLanguage[uiLanguage.value][activeTab.value])
const tabLabel = (tab: Tab) => tabLabelByLanguage[uiLanguage.value][tab]

const handleUiLanguageUpdated = (event: Event) => {
  const customEvent = event as CustomEvent<{ language?: unknown }>
  uiLanguage.value = normalizeUiLanguage(customEvent.detail?.language ?? getStoredUiLanguage())
}

const mcpRef  = ref<{ refresh: () => void } | null>(null)
const ragRef  = ref<{ refresh: () => void } | null>(null)

watch(() => props.modelValue, (visible) => {
  if (!visible) return
  ragRef.value?.refresh()
  mcpRef.value?.refresh()
})

watch(activeTab, (tab) => {
  if (tab === 'mcp') mcpRef.value?.refresh()
  if (tab === 'rag') ragRef.value?.refresh()
})

onMounted(() => {
  window.addEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
})

onUnmounted(() => {
  window.removeEventListener('ui-language-updated', handleUiLanguageUpdated as EventListener)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="backdrop">
      <div
        v-if="modelValue"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[3px] font-sans"
        @click.self="close"
      >
        <Transition name="modal">
          <div v-if="modelValue" class="flex w-[820px] max-w-[calc(100vw-48px)] h-[600px] max-h-[calc(100vh-80px)] bg-white dark:bg-[#252525] rounded-2xl overflow-hidden shadow-2xl border border-black/5 dark:border-white/5">

            <!-- Sidebar -->
            <aside class="w-[200px] shrink-0 bg-[#f9f9f8] dark:bg-[#1a1a1a] p-4 flex flex-col gap-1 border-r border-[#e5e5e5] dark:border-[#333]">
              <Button
                variant="ghost"
                size="icon-sm"
                class="mb-3 text-muted-foreground hover:bg-[#e5e5e5] dark:hover:bg-[#333]"
                @click="close"
                :title="closeTitle"
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="w-4 h-4">
                  <path d="M6 18L18 6M6 6l12 12"/>
                </svg>
              </Button>
              
              <nav class="flex flex-col gap-0.5">
                <Button
                  v-for="tab in tabs" :key="tab.id"
                  variant="ghost"
                  class="justify-start gap-2.5 px-3 py-2 text-left text-[0.92rem] font-medium"
                  :class="[
                    activeTab === tab.id 
                      ? 'bg-[#ebebeb] dark:bg-[#333] text-[#1a1a1a] dark:text-[#ececec]' 
                      : 'text-muted-foreground hover:bg-[#ebebeb]/60 dark:hover:bg-[#333]/60'
                  ]"
                  @click="activeTab = tab.id"
                >
                  <svg class="w-4 h-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path :d="tab.icon" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                  {{ tabLabel(tab.id) }}
                </Button>
              </nav>
            </aside>

            <!-- Main Content -->
            <main class="flex-1 px-8 py-7 overflow-y-auto custom-scrollbar relative">
              <h2 class="text-xl font-bold text-[#1a1a1a] dark:text-[#ececec] mb-6 tracking-tight">
                {{ currentTabTitle }}
              </h2>

              <div class="text-[#1a1a1a] dark:text-[#ececec]">
                <GeneralTab v-if="activeTab === 'general'" />
                <ModelTab   v-else-if="activeTab === 'model'" />
                <McpTab     v-else-if="activeTab === 'mcp'"   ref="mcpRef" />
                <RagTab     v-else-if="activeTab === 'rag'"   ref="ragRef" />
                <SkillTab   v-else-if="activeTab === 'skill'" />
                <MemoryTab  v-else-if="activeTab === 'memory'" />
                <DataTab    v-else-if="activeTab === 'data'" />
                <AboutTab   v-else-if="activeTab === 'about'" />
              </div>
            </main>

          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.backdrop-enter-active, .backdrop-leave-active { transition: opacity 0.2s ease; }
.backdrop-enter-from, .backdrop-leave-to { opacity: 0; }

.modal-enter-active { transition: all 0.25s cubic-bezier(0.34, 1.3, 0.64, 1); }
.modal-leave-active { transition: all 0.2s ease-in; }
.modal-enter-from { opacity: 0; transform: scale(0.96) translateY(10px); }
.modal-leave-to { opacity: 0; transform: scale(0.98); }

.custom-scrollbar::-webkit-scrollbar { width: 5px; }
.custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
.custom-scrollbar::-webkit-scrollbar-thumb { background: #d4d4d4; border-radius: 4px; }
.dark .custom-scrollbar::-webkit-scrollbar-thumb { background: #444; }
</style>
