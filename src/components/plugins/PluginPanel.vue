<script setup lang="ts">
// 插件界面宿主：sandbox iframe + postMessage 桥。
//
// 安全模型：
// - iframe 带 sandbox="allow-scripts"（无 allow-same-origin）→ 不透明源，
//   物理上无法访问宿主 DOM / localStorage / cookie。
// - 握手 token：宿主在收到 nova:ready 后生成随机 token 下发，
//   之后插件每次调用必须携带，伪造消息无效。
// - 通道白名单：只路由 nova:getSettings / nova:setSettings / nova:callTool。
// - 懒挂载：组件卸载即销毁 iframe，无任何残余。
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

const props = defineProps<{
  pluginId: string
  pluginName: string
  view: string
}>()

const frame = ref<HTMLIFrameElement | null>(null)
const loadFailed = ref(false)

// Windows(WebView2) 的自定义协议映射为 http://nova-plugin.localhost/...，
// macOS/Linux 使用 nova-plugin://localhost/...。
const pluginOrigin = (() => {
  const ua = navigator.userAgent.toLowerCase()
  return ua.includes('windows') ? 'http://nova-plugin.localhost' : 'nova-plugin://localhost'
})()

const frameSrc = computed(
  () => `${pluginOrigin}/${props.pluginId}/${props.view.replace(/^\/+/, '')}`,
)

let bridgeToken = ''
let settingsSnapshot: Record<string, unknown> = {}

const currentTheme = () =>
  document.documentElement.classList.contains('dark') ? 'dark' : 'light'

function reply(payload: Record<string, unknown>) {
  frame.value?.contentWindow?.postMessage(payload, '*')
}

function isBridgeMessage(data: unknown): data is { channel: string; token?: string; seq?: number } {
  if (!data || typeof data !== 'object') return false
  const channel = (data as Record<string, unknown>).channel
  return typeof channel === 'string'
}

async function onMessage(event: MessageEvent) {
  if (event.source !== frame.value?.contentWindow) return
  const msg = event.data
  if (!isBridgeMessage(msg)) return

  if (msg.channel === 'nova:ready') {
    // 握手：下发 token + 插件信息 + 设置快照 + 当前主题。
    const array = new Uint8Array(16)
    crypto.getRandomValues(array)
    bridgeToken = Array.from(array, (b) => b.toString(16).padStart(2, '0')).join('')
    try {
      settingsSnapshot = await invoke('get_plugin_settings', { pluginId: props.pluginId })
    } catch {
      settingsSnapshot = {}
    }
    reply({
      channel: 'nova:hello',
      token: bridgeToken,
      settings: settingsSnapshot,
      theme: currentTheme(),
      plugin: { id: props.pluginId, name: props.pluginName },
    })
    return
  }

  if (!bridgeToken || msg.token !== bridgeToken) return

  if (msg.channel === 'nova:getSettings') {
    try {
      const settings = await invoke('get_plugin_settings', { pluginId: props.pluginId })
      reply({ channel: 'nova:settings', seq: msg.seq, result: settings })
    } catch (e) {
      reply({ channel: 'nova:error', seq: msg.seq, error: String(e) })
    }
    return
  }

  if (msg.channel === 'nova:setSettings') {
    try {
      const settings = (msg as unknown as { settings: Record<string, unknown> }).settings
      await invoke('set_plugin_settings', { pluginId: props.pluginId, settings })
      settingsSnapshot = settings
      reply({ channel: 'nova:saved', seq: msg.seq, result: true })
    } catch (e) {
      reply({ channel: 'nova:error', seq: msg.seq, error: String(e) })
    }
    return
  }

  if (msg.channel === 'nova:callTool') {
    try {
      const { tool, args } = msg as unknown as { tool: string; args: Record<string, unknown> }
      const result = await invoke('call_plugin_tool', {
        pluginId: props.pluginId,
        tool,
        args,
      })
      reply({ channel: 'nova:toolResult', seq: msg.seq, result })
    } catch (e) {
      reply({ channel: 'nova:error', seq: msg.seq, error: String(e) })
    }
  }
}

// 主题跟随：监听 html.dark class 变化并推送给插件。
let themeObserver: MutationObserver | null = null

onMounted(async () => {
  window.addEventListener('message', onMessage as unknown as EventListener)
  themeObserver = new MutationObserver(() => {
    frame.value?.contentWindow?.postMessage(
      { channel: 'nova:theme', theme: currentTheme() },
      '*',
    )
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  })
})

onBeforeUnmount(() => {
  window.removeEventListener('message', onMessage as unknown as EventListener)
  themeObserver?.disconnect()
  themeObserver = null
  bridgeToken = ''
})
</script>

<template>
  <div class="w-full">
    <iframe
      ref="frame"
      :src="frameSrc"
      sandbox="allow-scripts"
      class="h-[min(640px,70vh)] w-full rounded-xl border border-[#e5e7eb] bg-white dark:border-[#333] dark:bg-[#242424]"
      :title="`插件界面：${pluginName}`"
      @error="loadFailed = true"
    />
    <p v-if="loadFailed" class="mt-2 text-[12px] text-destructive">
      插件界面加载失败，请确认插件目录中存在 {{ view }} 且插件已启用。
    </p>
  </div>
</template>
