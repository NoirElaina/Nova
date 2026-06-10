<script setup lang="ts">
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
</script>

<template>
  <Card class="border-[#e5e7eb] py-4 dark:border-[#333]">
    <CardHeader class="px-4 pb-2">
      <CardTitle class="flex items-center gap-2 text-3xl font-extrabold tracking-tighter">
        <svg class="text-primary" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 2v20M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/>
        </svg>
        Nova
      </CardTitle>
      <CardDescription class="font-mono text-xs">v0.1.0</CardDescription>
    </CardHeader>

    <CardContent class="space-y-4 px-4">
      <p class="max-w-[420px] text-[0.95rem] leading-relaxed text-[#6b7280] dark:text-[#a3a3a3]">
        基于 Tauri 2 + Vue 3 + TailwindCSS 构建的桌面 AI 助手，支持本地文件、Bash 与多 MCP Server 工具链。
      </p>

      <div class="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" as-child>
          <a href="#">GitHub</a>
        </Button>
        <Button variant="outline" size="sm" as-child>
          <a href="#">Document</a>
        </Button>
        <Button variant="outline" size="sm" as-child>
          <a href="#">Feedback</a>
        </Button>
      </div>

      <div class="flex flex-wrap gap-2">
        <span class="rounded-md bg-[#f3f4f6] px-2.5 py-1 text-xs font-medium text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#d1d5db]">Tauri v2</span>
        <span class="rounded-md bg-[#f3f4f6] px-2.5 py-1 text-xs font-medium text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#d1d5db]">Rust</span>
        <span class="rounded-md bg-[#f3f4f6] px-2.5 py-1 text-xs font-medium text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#d1d5db]">Vue 3</span>
        <span class="rounded-md bg-[#f3f4f6] px-2.5 py-1 text-xs font-medium text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#d1d5db]">TailwindCSS 4</span>
        <span class="rounded-md bg-[#f3f4f6] px-2.5 py-1 text-xs font-medium text-[#6b7280] dark:bg-[#2a2a2a] dark:text-[#d1d5db]">MCP Protocol</span>
      </div>
    </CardContent>
  </Card>
</template>
