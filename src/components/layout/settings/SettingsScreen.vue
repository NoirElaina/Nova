<script setup lang="ts">
import { ref, watch } from 'vue'
import { Button } from '@/components/ui/button'

import GeneralTab from './tabs/GeneralTab.vue'
import ModelTab   from './tabs/ModelTab.vue'
import McpTab     from './tabs/McpTab.vue'
import RagTab     from './tabs/RagTab.vue'
import SkillTab   from './tabs/SkillTab.vue'
import MemoryTab  from './tabs/MemoryTab.vue'
import DataTab    from './tabs/DataTab.vue'
import AboutTab   from './tabs/AboutTab.vue'

type MainView = 'chat' | 'settings'

const emit = defineEmits<{
  (e: 'change-main-view', view: MainView): void
}>()

type Tab = 'general' | 'model' | 'mcp' | 'rag' | 'skill' | 'memory' | 'data' | 'about'
const activeTab = ref<Tab>('general')

const mcpRef = ref<{ refresh: () => void } | null>(null)
const ragRef = ref<{ refresh: () => void } | null>(null)

watch(activeTab, (tab) => {
  if (tab === 'mcp') mcpRef.value?.refresh()
  if (tab === 'rag') ragRef.value?.refresh()
})

const tabs: { id: Tab; icon: string; labelZh: string }[] = [
  { id: 'general', icon: 'M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z M15 12a3 3 0 11-6 0 3 3 0 016 0z', labelZh: '通用' },
  { id: 'model', icon: 'M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z', labelZh: '模型' },
  { id: 'mcp', icon: 'M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z', labelZh: 'MCP' },
  { id: 'rag', icon: 'M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.746 0 3.332.477 4.5 1.253v13C19.832 18.477 18.246 18 16.5 18c-1.746 0-3.332.477-4.5 1.253', labelZh: 'RAG' },
  { id: 'skill', icon: 'M9.813 15.904A3 3 0 1012.087 18M5.143 4.567a3 3 0 103.707 3.707M18.36 5.143a3 3 0 10-3.707 3.707', labelZh: '技能' },
  { id: 'memory', icon: 'M9 12h6M9 16h6M9 8h6M6 3h12a2 2 0 012 2v14a2 2 0 01-2 2H6a2 2 0 01-2-2V5a2 2 0 012-2z', labelZh: '记忆' },
  { id: 'data', icon: 'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4', labelZh: '数据' },
  { id: 'about', icon: 'M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z', labelZh: '关于' },
]

const sidebarItemClass = 'h-8 w-full justify-start gap-2.5 rounded-md px-2.5 text-left text-[13px] font-normal transition-colors'
const sidebarItemActiveClass = 'bg-white text-[#111827] shadow-[0_1px_1px_rgba(15,23,42,0.04)] ring-1 ring-[#e5e7eb] dark:bg-[#2b2b2b] dark:text-[#f5f5f5] dark:ring-[#383838]'
const sidebarItemIdleClass = 'text-[#475569] hover:bg-white/70 hover:text-[#111827] dark:text-[#c8c8c8] dark:hover:bg-[#2a2a2a] dark:hover:text-[#f5f5f5]'
</script>

<template>
  <div class="flex h-full w-full bg-[#fcfcfc] dark:bg-[#1a1a1a]">
    <!-- 设置内部侧边栏 -->
    <aside class="w-[225px] flex-shrink-0 flex flex-col bg-[#f4f7fb] dark:bg-[#1f1f1f] border-r border-[#dfe6ee] dark:border-[#333] transition-all duration-300">
      <div class="flex flex-1 flex-col gap-0.5 overflow-y-auto p-4 custom-scrollbar">
        <!-- 返回按钮 -->
        <Button
          variant="ghost"
          class="mb-3 h-8 w-full justify-start gap-2.5 rounded-md px-2.5 text-left text-[13px] text-[#475569] hover:bg-white/70 hover:text-[#111827] dark:text-[#c8c8c8] dark:hover:bg-[#2a2a2a] dark:hover:text-[#f5f5f5]"
          @click="emit('change-main-view', 'chat')"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" class="h-4 w-4 shrink-0">
            <path d="M19 12H5M12 19l-7-7 7-7" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          返回聊天
        </Button>

        <!-- 设置标题 -->
        <div class="px-2.5 pb-1 pt-2 text-[11px] font-medium uppercase tracking-[0.04em] text-[#8a94a3] dark:text-[#858585]">
          设置
        </div>

        <!-- 导航项 -->
        <Button
          v-for="tab in tabs"
          :key="tab.id"
          variant="ghost"
          :class="[sidebarItemClass, activeTab === tab.id ? sidebarItemActiveClass : sidebarItemIdleClass]"
          @click="activeTab = tab.id"
        >
          <svg class="h-4 w-4 shrink-0 text-[#64748b]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <path :d="tab.icon" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          {{ tab.labelZh }}
        </Button>
      </div>

      <!-- 底部 Nova 品牌区 -->
      <div class="flex items-center gap-2 border-t border-[#dfe6ee] px-2.5 py-2 dark:border-[#333]">
        <div class="flex h-7 w-7 items-center justify-center rounded-full bg-[#2f343b] text-[13px] font-medium text-white">N</div>
        <span class="text-[13px] font-medium leading-tight text-[#111827] dark:text-[#ececec]">Nova</span>
      </div>
    </aside>

    <!-- 主内容区 -->
    <main class="flex-1 overflow-y-auto custom-scrollbar px-8 py-7">
      <h2 class="mb-6 text-xl font-bold text-[#1a1a1a] dark:text-[#ececec] tracking-tight">
        {{ tabs.find(t => t.id === activeTab)?.labelZh ?? activeTab }}
      </h2>

      <div class="text-[#1a1a1a] dark:text-[#ececec]">
        <GeneralTab v-if="activeTab === 'general'" />
        <ModelTab   v-else-if="activeTab === 'model'" />
        <McpTab     v-else-if="activeTab === 'mcp'" ref="mcpRef" />
        <RagTab     v-else-if="activeTab === 'rag'" ref="ragRef" />
        <SkillTab   v-else-if="activeTab === 'skill'" />
        <MemoryTab  v-else-if="activeTab === 'memory'" />
        <DataTab    v-else-if="activeTab === 'data'" />
        <AboutTab   v-else-if="activeTab === 'about'" />
      </div>
    </main>
  </div>
</template>

<style scoped>
.custom-scrollbar::-webkit-scrollbar { width: 6px; height: 6px; }
.custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
.custom-scrollbar::-webkit-scrollbar-thumb { background-color: #e5e5e5; border-radius: 10px; }
.dark .custom-scrollbar::-webkit-scrollbar-thumb { background-color: #444; }
</style>
