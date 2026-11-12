<script setup lang="ts">
import { ref } from 'vue'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

const checkingUpdate = ref(false)
const installingUpdate = ref(false)
const updateStatus = ref<string | null>(null)
const pendingVersion = ref<string | null>(null)
let pendingUpdateHandle: Awaited<ReturnType<typeof check>> = null

const handleCheckUpdate = async () => {
  if (checkingUpdate.value || installingUpdate.value) return
  checkingUpdate.value = true
  updateStatus.value = '正在检查 GitHub Releases 最新版本...'
  pendingVersion.value = null
  pendingUpdateHandle = null
  try {
    const update = await check()
    if (!update) {
      updateStatus.value = '当前已是最新版本（v0.1.0）。'
      return
    }
    pendingUpdateHandle = update
    pendingVersion.value = update.version
    updateStatus.value = `发现新版本 v${update.version}${update.body ? `：${update.body}` : ''}`
  } catch (error) {
    updateStatus.value = `检查更新失败：${error instanceof Error ? error.message : String(error)}`
  } finally {
    checkingUpdate.value = false
  }
}

const handleInstallUpdate = async () => {
  if (!pendingUpdateHandle || installingUpdate.value) return
  installingUpdate.value = true
  updateStatus.value = `正在下载并校验 v${pendingUpdateHandle.version} 更新包...`
  try {
    await pendingUpdateHandle.downloadAndInstall()
    updateStatus.value = '更新已安装，正在重启应用...'
    await relaunch()
  } catch (error) {
    updateStatus.value = `安装更新失败：${error instanceof Error ? error.message : String(error)}`
    installingUpdate.value = false
  }
}
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
      <p class="max-w-[460px] text-[0.95rem] leading-relaxed text-[#6b7280] dark:text-[#a3a3a3]">
        基于 Tauri 2 + Vue 3 + TailwindCSS 构建的桌面 AI 助手，支持本地文件、交互式 PTY 终端与多 MCP Server 工具链。
      </p>

      <div class="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" as-child>
          <a href="https://github.com/NoirElaina/Nova" target="_blank" rel="noreferrer">GitHub</a>
        </Button>
        <Button variant="outline" size="sm" as-child>
          <a href="https://github.com/NoirElaina/Nova/releases" target="_blank" rel="noreferrer">Releases</a>
        </Button>
        <Button variant="outline" size="sm" as-child>
          <a href="https://github.com/NoirElaina/Nova/issues" target="_blank" rel="noreferrer">Feedback</a>
        </Button>
        <Button
          variant="secondary"
          size="sm"
          :disabled="checkingUpdate || installingUpdate"
          @click="handleCheckUpdate"
        >
          {{ checkingUpdate ? '检查中...' : '检查更新' }}
        </Button>
        <Button
          v-if="pendingVersion"
          size="sm"
          :disabled="installingUpdate"
          @click="handleInstallUpdate"
        >
          {{ installingUpdate ? '安装中...' : `下载并更新 v${pendingVersion}` }}
        </Button>
      </div>

      <p
        v-if="updateStatus"
        class="rounded-md border border-[#e5e7eb] bg-[#f9fafb] px-3 py-2 text-xs text-[#374151] dark:border-[#333] dark:bg-[#1a1a1a] dark:text-[#d1d5db]"
      >
        {{ updateStatus }}
      </p>

      <div class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs leading-relaxed text-[#6b7280] dark:text-[#a3a3a3]">
        更新包采用 Tauri Ed25519 公钥离线验签（托管于 GitHub Releases）；未配置 Windows Authenticode EV 签名时，首次手动运行安装包可能会出现 SmartScreen 提示，可点击“更多信息 → 仍要运行”。
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
