<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

type MCPServerConfig = { type: 'stdio'; command: string; args: string[]; env?: Record<string, string> } | { type: 'sse'; url: string; headers?: Record<string, string> } | { type: 'streamable_http'; url: string; headers?: Record<string, string> }
type MCPServerEntry = { name: string; enabled: boolean; config: MCPServerConfig }
type ServerStatus = { name: string; status: 'connected' | 'error' | 'connecting' | 'disconnected'; type: 'stdio' | 'sse' | 'streamable_http'; enabled: boolean; toolCount?: number; error?: string }
type MCPForm = { name: string; type: 'stdio' | 'sse' | 'streamable_http'; command: string; args: string; env: string; url: string; headers: string }
type ToastItem = { id: number; message: string; variant: 'error' | 'success' }

const addServer = async (name: string, config: MCPServerConfig) => {
  await invoke('add_mcp_server', { name, config })
}
const getServer = async (name: string): Promise<MCPServerEntry> => {
  return await invoke('get_mcp_server', { name })
}
const updateServer = async (oldName: string, newName: string, config: MCPServerConfig) => {
  await invoke('update_mcp_server', { oldName, newName, config })
}
const removeServer = async (name: string) => {
  await invoke('remove_mcp_server', { name })
}
const getServerStatuses = async (): Promise<ServerStatus[]> => {
  return await invoke('get_mcp_server_statuses')
}
const reloadAllServers = async () => {
  await invoke('reload_all_mcp_servers')
}
const setServerEnabled = async (name: string, enabled: boolean) => {
  await invoke('set_mcp_server_enabled', { name, enabled })
}

const servers = ref<ServerStatus[]>([])
const loading = ref(false)
const adding = ref(false)
const reloading = ref(false)
const error = ref('')
const toasts = ref<ToastItem[]>([])
const showForm = ref(false)
const editingOriginalName = ref<string | null>(null)
const editingEnabled = ref(true)
const loadingEditName = ref<string | null>(null)
const removingName = ref<string | null>(null)
const togglingName = ref<string | null>(null)
const form = ref<MCPForm>({ name: '', type: 'stdio', command: 'npx', args: '-y @playwright/mcp@latest', env: '', url: '', headers: '' })

const pushToast = (message: string, variant: ToastItem['variant']) => {
  const id = Date.now() + Math.floor(Math.random() * 1000)
  toasts.value.push({ id, message, variant })
  window.setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }, 3500)
}

const parseKeyValueLines = (value: string) => {
  const result: Record<string, string> = {}
  value.trim().split('\n').forEach(line => {
    const eq = line.indexOf('=')
    if (eq > 0) result[line.slice(0, eq).trim()] = line.slice(eq + 1).trim()
  })
  return result
}

const keyValueText = (value?: Record<string, string>) => {
  if (!value) return ''
  return Object.entries(value).map(([key, item]) => `${key}=${item}`).join('\n')
}

const resetForm = () => {
  form.value = { name: '', type: 'stdio', command: 'npx', args: '-y @playwright/mcp@latest', env: '', url: '', headers: '' }
  editingOriginalName.value = null
  editingEnabled.value = true
  error.value = ''
  showForm.value = false
}

const openCreateForm = () => {
  if (showForm.value && !editingOriginalName.value) {
    resetForm()
    return
  }
  resetForm()
  showForm.value = true
}

const fillFormFromServer = (server: MCPServerEntry) => {
  const { config } = server
  editingOriginalName.value = server.name
  editingEnabled.value = server.enabled

  if (config.type === 'stdio') {
    const env = config.env
      ? Object.entries(config.env).map(([key, value]) => `${key}=${value}`).join('\n')
      : ''
    form.value = {
      name: server.name,
      type: 'stdio',
      command: config.command,
      args: config.args.join(' '),
      env,
      url: '',
      headers: ''
    }
    return
  }

  form.value = {
    name: server.name,
    type: config.type,
    command: '',
    args: '',
    env: '',
    url: config.url,
    headers: keyValueText(config.headers)
  }
}

const refresh = async () => {
  loading.value = true
  try { servers.value = await getServerStatuses() }
  catch (e) {
    servers.value = []
    console.error('Failed to load MCP statuses:', e)
  }
  finally { loading.value = false }
}

const handleEdit = async (name: string) => {
  loadingEditName.value = name
  error.value = ''
  try {
    const server = await getServer(name)
    fillFormFromServer(server)
    showForm.value = true
  }
  catch (e) {
    console.error(`Failed to load MCP config (${name}):`, e)
  }
  finally {
    loadingEditName.value = null
  }
}

const submit = async () => {
  const name = form.value.name.trim()
  if (!name) { error.value = '请填写名称'; return }
  const isEditing = Boolean(editingOriginalName.value)
  const originalName = editingOriginalName.value
  const wasEnabled = editingEnabled.value
  let config: MCPServerConfig
  if (form.value.type === 'stdio') {
    if (!form.value.command.trim()) { error.value = '请填写命令'; return }
    const args = form.value.args.trim() ? form.value.args.trim().split(/\s+/) : []
    const env = parseKeyValueLines(form.value.env)
    config = { type: 'stdio', command: form.value.command.trim(), args, ...(Object.keys(env).length ? { env } : {}) }
  } else {
    if (!form.value.url.trim()) { error.value = '请填写 URL'; return }
    const headers = parseKeyValueLines(form.value.headers)
    config = form.value.type === 'sse'
      ? { type: 'sse', url: form.value.url.trim(), ...(Object.keys(headers).length ? { headers } : {}) }
      : { type: 'streamable_http', url: form.value.url.trim(), ...(Object.keys(headers).length ? { headers } : {}) }
  }
  adding.value = true; error.value = ''
  try {
    if (isEditing && originalName) {
      await updateServer(originalName, name, config)
    } else {
      await addServer(name, config)
    }
    resetForm()
    await refresh()
    pushToast(isEditing
      ? `MCP 服务已更新${wasEnabled ? '并重新连接' : ''}。`
      : 'MCP 服务已添加并触发连接。', 'success')
  }
  catch (e) {
    console.error('Failed to submit MCP server:', e)
  }
  finally { adding.value = false }
}

const handleRemove = async (name: string) => {
  removingName.value = name
  try {
    await removeServer(name)
    await refresh()
    pushToast(`已删除 MCP 服务: ${name}`, 'success')
  }
  catch (e) {
    console.error(`Failed to remove MCP server (${name}):`, e)
  }
  finally { removingName.value = null }
}

const handleReload = async () => {
  reloading.value = true
  try {
    await reloadAllServers()
    await refresh()
    pushToast('MCP 服务重连完成。', 'success')
  }
  catch (e) {
    console.error('Failed to reload MCP servers:', e)
  }
  finally { reloading.value = false }
}

const handleToggleEnabled = async (name: string, enabled: boolean) => {
  togglingName.value = name
  try {
    await setServerEnabled(name, enabled)
    await refresh()
    pushToast(`${enabled ? '已启用' : '已停用'} MCP 服务: ${name}`, 'success')
  }
  catch (e) {
    console.error(`Failed to toggle MCP server (${name}):`, e)
  }
  finally { togglingName.value = null }
}

defineExpose({ refresh })
refresh()
</script>

<template>
  <div class="px-6 py-4 flex flex-col h-full overflow-y-auto">
    <TransitionGroup
      name="mcp-toast"
      tag="div"
      class="fixed top-5 right-5 z-[80] flex flex-col gap-2 pointer-events-none"
    >
      <div
        v-for="toast in toasts"
        :key="toast.id"
        class="min-w-[260px] max-w-[360px] px-4 py-3 rounded-lg border shadow-[0_8px_20px_rgba(0,0,0,0.12)] text-[13px] leading-relaxed pointer-events-auto"
        :class="toast.variant === 'error'
          ? 'bg-[#fff4f4] dark:bg-[#3a2222] border-[#f2c9c9] dark:border-[#6a3535] text-[#9f2f2f] dark:text-[#ffb3b3]'
          : 'bg-[#f2fbf4] dark:bg-[#1f3325] border-[#cde8d3] dark:border-[#3a6b48] text-[#1f6a34] dark:text-[#9ae2ad]'"
      >
        {{ toast.message }}
      </div>
    </TransitionGroup>

    <div class="flex items-center justify-between mb-4">
      <span class="text-[12.5px] text-[#64748b] dark:text-[#a3a3a3]">{{ servers.length }} 个服务</span>
      <div class="flex items-center gap-2">
        <Button variant="ghost" size="sm" class="gap-[6px] text-[13px] text-[#475569] hover:bg-[#f3f4f6] hover:text-[#111827] dark:text-[#a3a3a3] dark:hover:bg-[#2a2a2a] dark:hover:text-[#f3f4f6]" :disabled="reloading" @click="handleReload">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="w-[15px] h-[15px]" :class="{ 'animate-spin': reloading }">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          重新连接
        </Button>
        <Button size="sm" @click="openCreateForm">
          {{ showForm && !editingOriginalName ? '取消' : '+ 添加' }}
        </Button>
      </div>
    </div>

    <div v-if="showForm" class="bg-[#f9fafb] dark:bg-[#1e1e1e] border border-[#e5e7eb] dark:border-[#333] rounded-xl p-4 mb-4 flex flex-col transition-all overflow-hidden origin-top">
      <div class="flex items-center justify-between mb-3">
        <div>
          <div class="text-[14px] font-semibold text-[#111827] dark:text-[#f3f4f6]">
            {{ editingOriginalName ? '编辑 MCP 服务' : '添加 MCP 服务' }}
          </div>
          <div class="text-[12px] text-[#6b7280] dark:text-[#a3a3a3] mt-0.5">
            {{ editingOriginalName ? '修改后会覆盖当前配置，并在已启用时自动重连。' : '保存后会立即写入配置，并尝试连接服务。' }}
          </div>
        </div>
        <span v-if="editingOriginalName" class="text-[11px] px-2 py-1 rounded bg-[#f3f4f6] dark:bg-[#2a2a2a] text-[#6b7280] dark:text-[#a3a3a3] font-mono">
          原名称: {{ editingOriginalName }}
        </span>
      </div>
      <div class="flex gap-3 items-end mb-2.5">
        <div class="flex-1 flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">名称</label>
          <Input v-model="form.name" placeholder="filesystem" class="w-full h-9 px-3 text-[14px]"/>
        </div>
        <div class="w-[210px] flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">类型</label>
          <div class="flex p-[2px] bg-muted dark:bg-[#2a2a2a] rounded-[8px]">
            <Button variant="ghost" size="sm" class="h-auto flex-1 py-[5px] text-[12px]" :class="{ 'bg-background text-foreground shadow-[0_1px_3px_rgba(0,0,0,0.05)] dark:bg-[#333]': form.type === 'stdio' }" @click="form.type = 'stdio'">stdio</Button>
            <Button variant="ghost" size="sm" class="h-auto flex-1 py-[5px] text-[12px]" :class="{ 'bg-background text-foreground shadow-[0_1px_3px_rgba(0,0,0,0.05)] dark:bg-[#333]': form.type === 'sse' }" @click="form.type = 'sse'">SSE</Button>
            <Button variant="ghost" size="sm" class="h-auto flex-1 py-[5px] text-[12px]" :class="{ 'bg-background text-foreground shadow-[0_1px_3px_rgba(0,0,0,0.05)] dark:bg-[#333]': form.type === 'streamable_http' }" @click="form.type = 'streamable_http'">streamable_http</Button>
          </div>
        </div>
      </div>
      <template v-if="form.type === 'stdio'">
        <div class="mb-2.5 flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">命令</label>
          <Input v-model="form.command" placeholder="npx / uvx / node" class="w-full h-9 px-3 text-[14px] font-mono"/>
        </div>
        <div class="mb-2.5 flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">参数 <span class="font-normal text-[#64748b] dark:text-[#a3a3a3] ml-1 lowercase">空格分隔</span></label>
          <Input v-model="form.args" placeholder="-y @playwright/mcp@latest" class="w-full h-9 px-3 text-[14px] font-mono"/>
        </div>
        <div class="mb-2.5 flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">环境变量 <span class="font-normal text-[#64748b] dark:text-[#a3a3a3] ml-1 lowercase">每行 KEY=VALUE（可选）</span></label>
          <Textarea v-model="form.env" placeholder="API_KEY=xxx" rows="2" class="w-full px-3 py-2 text-[14px] font-mono resize-y"/>
        </div>
      </template>
      <template v-else>
        <div class="mb-2.5 flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">{{ form.type === 'sse' ? 'SSE URL' : 'HTTP URL' }}</label>
          <Input v-model="form.url" :placeholder="form.type === 'sse' ? 'http://localhost:8080/sse' : 'https://example.com/mcp'" class="w-full h-9 px-3 text-[14px] font-mono"/>
        </div>
        <div class="mb-2.5 flex flex-col text-[14px]">
          <label class="text-[13px] font-semibold text-[#111827] dark:text-[#f3f4f6] mb-[6px] uppercase tracking-wider">请求头 <span class="font-normal text-[#64748b] dark:text-[#a3a3a3] ml-1 lowercase">每行 KEY=VALUE（可选）</span></label>
          <Textarea v-model="form.headers" placeholder="Authorization=Bearer xxx&#10;X-API-Key=xxx" rows="3" class="w-full px-3 py-2 text-[14px] font-mono resize-y"/>
          <div class="mt-1 text-[12px] text-[#6b7280] dark:text-[#a3a3a3]">保存时后端会加密这些值，连接时自动带上。</div>
        </div>
      </template>
      <div v-if="error" class="text-[12.5px] text-red-600 dark:text-red-400 mb-2.5">{{ error }}</div>
      <div class="flex items-center justify-end gap-3 mt-2">
        <Button @click="submit" :disabled="adding">
          {{ adding ? (editingOriginalName ? '保存中...' : '连接中...') : (editingOriginalName ? '保存修改' : '添加并连接') }}
        </Button>
        <Button variant="outline" size="sm" @click="resetForm">取消</Button>
      </div>
    </div>

    <div v-if="loading" class="text-center py-8 text-[13.5px] text-[#64748b] dark:text-[#a3a3a3]">加载中...</div>
    <div v-else-if="servers.length === 0 && !showForm" class="text-center py-8 text-[13.5px] text-[#64748b] dark:text-[#a3a3a3]">暂无 MCP 服务，点击「添加」接入工具服务</div>
    <div v-else class="flex flex-col gap-2">
      <div v-for="s in servers" :key="s.name" class="flex items-center justify-between p-3 border border-[#e5e7eb] dark:border-[#333] rounded-xl gap-3 transition-colors duration-150 hover:border-[#d1d5db] dark:hover:border-[#555]">
        <div class="flex items-center gap-2.5 flex-1 min-w-0">
          <span class="w-2 h-2 rounded-full shrink-0" :class="{
            'bg-[#4caf50]': s.status === 'connected',
            'bg-[#e53935]': s.status === 'error',
            'bg-[#fb8c00]': s.status === 'connecting',
            'bg-[#bdbdbd] dark:bg-[#666]': s.status === 'disconnected'
          }"></span>
          <div class="min-w-0">
            <div class="text-[13.5px] font-semibold text-[#111827] dark:text-[#f3f4f6] truncate">{{ s.name }}</div>
            <div class="flex items-center gap-2 mt-0.5">
              <span class="text-[11px] px-1.5 py-[1px] rounded bg-[#f3f4f6] dark:bg-[#2a2a2a] text-[#6b7280] dark:text-[#a3a3a3] font-mono shrink-0">{{ s.type }}</span>
              <span v-if="s.enabled" class="text-[11px] px-1.5 py-[1px] rounded bg-green-50 dark:bg-green-950/30 text-green-700 dark:text-green-400 shrink-0">已启用</span>
              <span v-else class="text-[11px] px-1.5 py-[1px] rounded bg-[#f3f4f6] dark:bg-[#2a2a2a] text-[#6b7280] dark:text-[#9f9f9f] shrink-0">已停用</span>
              <span v-if="s.status === 'connected'" class="text-[12px] text-[#6b7280] dark:text-[#a3a3a3] whitespace-nowrap shrink-0">{{ s.toolCount }} 个工具</span>
              <span v-if="s.error" class="text-[12px] text-red-600 dark:text-red-400 truncate" :title="s.error">{{ s.error }}</span>
            </div>
          </div>
        </div>
        <div class="flex items-center gap-2 shrink-0">
          <Button variant="outline" size="sm" class="px-3 py-1.5 text-[12px]" :disabled="loadingEditName === s.name" @click="handleEdit(s.name)">
            {{ loadingEditName === s.name ? '读取中...' : '编辑' }}
          </Button>
          <Button variant="outline" size="sm" class="px-3 py-1.5 text-[12px]" :class="s.enabled ? 'text-amber-600 dark:text-amber-400 border-amber-200 dark:border-amber-800 hover:bg-amber-50 dark:hover:bg-amber-950/30' : 'text-green-600 dark:text-green-400 border-green-200 dark:border-green-800 hover:bg-green-50 dark:hover:bg-green-950/30'" :disabled="togglingName === s.name" @click="handleToggleEnabled(s.name, !s.enabled)">
            {{ togglingName === s.name ? '处理中...' : (s.enabled ? '停用' : '启用') }}
          </Button>
          <Button variant="ghost" size="sm" class="px-3 py-1.5 text-[12.5px] text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-950/30" :disabled="removingName === s.name" @click="handleRemove(s.name)">
            {{ removingName === s.name ? '删除中...' : '删除' }}
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mcp-toast-enter-active,
.mcp-toast-leave-active {
  transition: all 0.22s ease;
}

.mcp-toast-enter-from,
.mcp-toast-leave-to {
  opacity: 0;
  transform: translateY(-8px) translateX(8px);
}
</style>
