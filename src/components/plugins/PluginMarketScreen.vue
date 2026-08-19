<script setup lang="ts">
// 插件市场/管理页（主界面视图，替代原 Agent 市场占位页）。
// 两级视图：grid = 插件卡片网格；detail = 单插件详情（元信息 + 启停 + iframe 配置页）。
// 生命周期闭环：zip 安装 / 卸载 / 检查更新（updateUrl）。
// 目录监听：后端 plugins-changed 事件推送时自动刷新列表（开发热改即时生效）。
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import ConfirmDialog from "@/components/ui/confirm-dialog/ConfirmDialog.vue";
import PluginPanel from "@/components/plugins/PluginPanel.vue";
import { emitToast, emitErrorToast } from "@/lib/toast";

type MainView = "chat" | "hooks" | "agent" | "plugins" | "schedule" | "settings";

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
}>();

type PluginTool = { name: string; description: string };
type PluginCommand = { name: string; title: string; description: string; promptTemplate: string };

type PluginInfo = {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  permissions: string[];
  tools: PluginTool[];
  commands: PluginCommand[];
  promptSection: { content: string; placement: string } | null;
  settingsTab: { title: string; icon: string; view: string } | null;
  system: boolean;
  updateUrl: string | null;
  enabled: boolean;
  dir: string;
  error: string | null;
};

const loading = ref(false);
const error = ref("");
const plugins = ref<PluginInfo[]>([]);
const view = ref<"grid" | "detail">("grid");
const selectedId = ref("");
const togglingId = ref("");
const installing = ref(false);

// 启用带权限声明的插件时弹一次确认。
const confirmTarget = ref<PluginInfo | null>(null);
const confirmVisible = ref(false);

// 卸载确认。
const uninstallTarget = ref<PluginInfo | null>(null);
const uninstallVisible = ref(false);
const uninstalling = ref(false);

// 更新检查结果确认。
const updateCheckState = ref<"" | "checking" | "done">("");
const updateInfo = ref<{ hasUpdate: boolean; currentVersion: string; remoteVersion: string } | null>(null);
const updateConfirmVisible = ref(false);
const updating = ref(false);

// 开发指南展开状态。
const guideOpen = ref(false);

const selectedPlugin = computed(
  () => plugins.value.find((p) => p.id === selectedId.value) ?? null,
);

const enabledCount = computed(() => plugins.value.filter((p) => p.enabled && !p.error).length);

const refresh = async () => {
  loading.value = true;
  error.value = "";
  try {
    plugins.value = await invoke<PluginInfo[]>("list_plugins");
  } catch (e) {
    error.value = String(e);
    plugins.value = [];
  } finally {
    loading.value = false;
  }
};

const openDetail = (plugin: PluginInfo) => {
  selectedId.value = plugin.id;
  updateCheckState.value = "";
  updateInfo.value = null;
  view.value = "detail";
};

const backToGrid = () => {
  view.value = "grid";
  selectedId.value = "";
};

const requestToggle = (plugin: PluginInfo) => {
  if (plugin.error) return;
  if (plugin.enabled || plugin.permissions.length === 0) {
    void applyToggle(plugin, !plugin.enabled);
    return;
  }
  confirmTarget.value = plugin;
  confirmVisible.value = true;
};

const applyToggle = async (plugin: PluginInfo, enabled: boolean) => {
  togglingId.value = plugin.id;
  error.value = "";
  try {
    await invoke("set_plugin_enabled", { pluginId: plugin.id, enabled });
    plugins.value = await invoke<PluginInfo[]>("list_plugins");
  } catch (e) {
    error.value = String(e);
  } finally {
    togglingId.value = "";
  }
};

const onConfirmEnable = () => {
  const plugin = confirmTarget.value;
  confirmVisible.value = false;
  if (!plugin) return;
  void applyToggle(plugin, true);
};

const openPluginsDir = async () => {
  try {
    await invoke("open_plugins_dir");
  } catch (e) {
    error.value = String(e);
  }
};

// zip 安装：文件选择 → 后端校验（manifest / 路径穿越 / 重名冲突）→ 移入 plugins/。
const installFromZip = async () => {
  if (installing.value) return;
  const picked = await openFileDialog({
    multiple: false,
    filters: [{ name: "Nova 插件包", extensions: ["zip"] }],
  });
  if (!picked || typeof picked !== "string") return;
  installing.value = true;
  error.value = "";
  try {
    const installed = await invoke<PluginInfo>("install_plugin_from_zip", { zipPath: picked });
    emitToast({ message: `已安装插件「${installed.name}」` });
    await refresh();
    openDetail(installed);
  } catch (e) {
    emitErrorToast("安装插件", e);
  } finally {
    installing.value = false;
  }
};

// 卸载：确认弹窗展示将删除的目录，系统插件拒绝。
const requestUninstall = (plugin: PluginInfo) => {
  if (plugin.system) {
    emitToast({ message: "系统内置插件不可卸载，只可停用" });
    return;
  }
  uninstallTarget.value = plugin;
  uninstallVisible.value = true;
};

const onConfirmUninstall = async () => {
  const plugin = uninstallTarget.value;
  uninstallVisible.value = false;
  if (!plugin) return;
  uninstalling.value = true;
  try {
    await invoke("uninstall_plugin", { pluginId: plugin.id });
    emitToast({ message: `已卸载「${plugin.name}」` });
    await refresh();
    backToGrid();
  } catch (e) {
    emitErrorToast("卸载插件", e);
  } finally {
    uninstalling.value = false;
  }
};

// 检查更新：下载 updateUrl 的 zip 比较版本号。
const checkUpdate = async () => {
  const plugin = selectedPlugin.value;
  if (!plugin || !plugin.updateUrl || updateCheckState.value === "checking") return;
  updateCheckState.value = "checking";
  updateInfo.value = null;
  try {
    const result = await invoke<{ hasUpdate: boolean; currentVersion: string; remoteVersion: string }>(
      "check_plugin_update",
      { pluginId: plugin.id },
    );
    updateInfo.value = result;
    updateCheckState.value = "done";
    if (result.hasUpdate) {
      updateConfirmVisible.value = true;
    } else {
      emitToast({ message: `已是最新版本（${result.currentVersion || "未知版本"}）` });
    }
  } catch (e) {
    updateCheckState.value = "done";
    emitErrorToast("检查更新", e);
  }
};

const onConfirmUpdate = async () => {
  const plugin = selectedPlugin.value;
  updateConfirmVisible.value = false;
  if (!plugin) return;
  updating.value = true;
  try {
    await invoke("update_plugin", { pluginId: plugin.id });
    emitToast({ message: "插件已更新" });
    updateCheckState.value = "";
    updateInfo.value = null;
    await refresh();
  } catch (e) {
    emitErrorToast("更新插件", e);
  } finally {
    updating.value = false;
  }
};

const backToChat = () => emit("change-main-view", "chat");

// 后端目录监听推送的插件变化 → 自动刷新（开发热改 / 手动增删目录）。
let unlistenPluginsChanged: UnlistenFn | null = null;

onMounted(() => {
  void refresh();
  void listen("plugins-changed", () => {
    void refresh();
  }).then((unlisten) => {
    unlistenPluginsChanged = unlisten;
  });
});

onUnmounted(() => {
  if (unlistenPluginsChanged) {
    unlistenPluginsChanged();
    unlistenPluginsChanged = null;
  }
});

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const dangerButtonClass =
  "h-8 border border-[#fecaca] bg-white px-3 text-[13px] text-[#dc2626] shadow-none hover:bg-[#fef2f2] dark:border-[#513030] dark:bg-[#242424] dark:text-[#fca5a5] dark:hover:bg-[#2d2d2d]";
const valueClass =
  "rounded-md border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2 text-[13px] text-[#1f2937] dark:border-[#333] dark:bg-[#262626] dark:text-[#e5e7eb]";
</script>

<template>
  <div :class="pageClass">
    <!-- 网格视图 -->
    <template v-if="view === 'grid'">
      <header class="flex flex-wrap items-start justify-between gap-3">
        <div class="space-y-1">
          <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">插件</h2>
          <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
            安装在 plugins 目录下的扩展：为 AI 增加工具、命令与提示词片段。已启用 {{ enabledCount }} / {{ plugins.length }}。
          </p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" :class="headerButtonClass" @click="backToChat">
            返回聊天
          </Button>
          <Button
            variant="ghost"
            size="sm"
            :class="headerButtonClass"
            :disabled="installing"
            @click="installFromZip"
          >
            {{ installing ? '安装中...' : '安装插件（zip）' }}
          </Button>
          <Button variant="ghost" size="sm" :class="headerButtonClass" @click="openPluginsDir">
            打开插件目录
          </Button>
          <Button
            variant="ghost"
            size="sm"
            :class="headerButtonClass"
            :disabled="loading"
            @click="refresh"
          >
            刷新
          </Button>
        </div>
      </header>

      <div v-if="error" class="text-[13px] text-[#dc2626] dark:text-[#fca5a5]">{{ error }}</div>

      <Card v-if="loading" :class="panelClass">
        <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在扫描插件...</CardContent>
      </Card>

      <Card v-else-if="plugins.length === 0" :class="panelClass">
        <CardContent class="space-y-1 px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">
          <p>暂无插件。点击「安装插件（zip）」或手动把插件目录放入应用数据目录的 plugins 子目录。</p>
          <p class="text-[12.5px]">JS 工具运行在零权限沙箱中，网络访问需在 manifest 声明 net: 权限。</p>
        </CardContent>
      </Card>

      <div v-else class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
        <button
          v-for="plugin in plugins"
          :key="plugin.id"
          type="button"
          class="flex min-h-[120px] cursor-pointer flex-col rounded-xl border p-4 text-left transition-all"
          :class="plugin.error
            ? 'border-[#fecaca] bg-white hover:border-[#dc2626]/60 dark:border-[#513030] dark:bg-[#242424]'
            : 'border-[#e5e7eb] bg-white hover:border-[#2563eb]/60 hover:shadow-[0_6px_20px_rgba(37,99,235,0.10)] dark:border-[#333] dark:bg-[#242424] dark:hover:border-[#60a5fa]/50 dark:hover:shadow-none'"
          :title="plugin.error ? '查看错误详情' : `查看「${plugin.name}」`"
          @click="openDetail(plugin)"
        >
          <div class="flex items-center justify-between gap-2">
            <span class="truncate text-[13.5px] font-semibold text-[#111827] dark:text-[#f3f4f6]">{{ plugin.name }}</span>
            <span
              v-if="plugin.error"
              class="shrink-0 rounded-full bg-red-50 px-2 py-0.5 text-[10.5px] font-medium text-red-600 dark:bg-red-950/30 dark:text-red-400"
            >加载失败</span>
            <span
              v-else-if="plugin.enabled"
              class="inline-flex shrink-0 items-center gap-1 rounded-full bg-[#eff6ff] px-2 py-0.5 text-[10.5px] font-medium text-[#1d4ed8] dark:bg-[#1e293b] dark:text-[#93c5fd]"
            >
              <span class="h-1.5 w-1.5 rounded-full bg-[#2563eb]"></span>
              已启用
            </span>
            <span
              v-else
              class="shrink-0 rounded-full bg-[#f1f5f9] px-2 py-0.5 text-[10.5px] text-[#94a3b8] dark:bg-[#2a2a2a] dark:text-[#8b8b8b]"
            >未启用</span>
          </div>
          <p class="mt-1.5 line-clamp-2 flex-1 break-words text-[12px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">
            {{ plugin.error ?? plugin.description }}
          </p>
          <div class="mt-2 flex flex-wrap items-center gap-1.5 text-[11px]">
            <span
              v-for="tool in plugin.tools.slice(0, 3)"
              :key="tool.name"
              class="rounded bg-[#eff6ff] px-1.5 py-0.5 font-mono text-[#1d4ed8] dark:bg-[#1e293b] dark:text-[#93c5fd]"
              :title="tool.description"
            >{{ tool.name }}</span>
            <span
              v-for="command in plugin.commands.slice(0, 2)"
              :key="command.name"
              class="rounded bg-emerald-50 px-1.5 py-0.5 font-mono text-emerald-700 dark:bg-emerald-950/30 dark:text-emerald-400"
              :title="command.description"
            >/{{ command.name }}</span>
            <span
              v-if="plugin.tools.length > 3"
              class="text-[#94a3b8] dark:text-[#8b8b8b]"
            >+{{ plugin.tools.length - 3 }}</span>
            <span
              v-if="!plugin.error && plugin.permissions.length > 0"
              class="rounded bg-amber-50 px-1.5 py-0.5 font-mono text-amber-700 dark:bg-amber-950/30 dark:text-amber-400"
              :title="`权限：${plugin.permissions.join('、')}`"
            >{{ plugin.permissions.length }} 项权限</span>
          </div>
        </button>

        <!-- 开发指南（可展开） -->
        <div class="flex min-h-[120px] flex-col rounded-xl border border-dashed border-[#c7d4e8] bg-white/40 p-4 text-[12px] leading-5 text-[#64748b] dark:border-[#3f3f3f] dark:bg-transparent dark:text-[#a3a3a3]">
          <button
            type="button"
            class="flex cursor-pointer items-center justify-between text-left"
            @click="guideOpen = !guideOpen"
          >
            <span class="text-[13px] font-medium text-[#475569] dark:text-[#d7d7d7]">开发指南</span>
            <span class="text-[#94a3b8] dark:text-[#8b8b8b]">{{ guideOpen ? '收起 ▲' : '展开 ▼' }}</span>
          </button>
          <div v-if="!guideOpen" class="mt-1.5">点击展开：manifest 字段、nova.* API 全量参考与安装结构。</div>
          <div v-else class="mt-2 space-y-2">
            <p><b>目录结构</b>：plugin.json（清单）+ main.js（沙箱入口，nova.tool 注册工具）+ ui/（界面文件）。</p>
            <p><b>manifest 贡献点</b>：tools（AI 工具）、settingsTab（设置页）、commands（斜杠命令）、promptSection（系统提示词片段）、updateUrl（更新源）、system（内置标记）。</p>
            <p><b>nova.* 沙箱 API</b>：tool / log / getSetting / setSetting / getSettings / http.get / http.postJson / usage.getTotal / usage.getToday / usage.getRecent(n) / session.getInfo / session.listTools / host.getTheme。</p>
            <p><b>热开发</b>：直接修改已启用插件的 main.js 保存即可，目录监听自动重载（下次工具调用执行新代码），无需重启应用。</p>
            <p><b>分发</b>：把插件目录打成 zip（根目录含 plugin.json），在「安装插件（zip）」中选择即装。</p>
          </div>
        </div>
      </div>
    </template>

    <!-- 详情视图 -->
    <template v-else-if="selectedPlugin">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex min-w-0 items-center gap-2.5">
          <Button variant="ghost" size="sm" :class="headerButtonClass" @click="backToGrid">
            ← 返回
          </Button>
          <h2 class="truncate text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">
            {{ selectedPlugin.name }}
          </h2>
          <span
            class="shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium"
            :class="selectedPlugin.error
              ? 'bg-red-50 text-red-600 dark:bg-red-950/30 dark:text-red-400'
              : selectedPlugin.enabled
                ? 'bg-[#eff6ff] text-[#1d4ed8] dark:bg-[#1e293b] dark:text-[#93c5fd]'
                : 'bg-[#f1f5f9] text-[#94a3b8] dark:bg-[#2a2a2a] dark:text-[#8b8b8b]'"
          >{{ selectedPlugin.error ? '加载失败' : selectedPlugin.enabled ? '已启用' : '未启用' }}</span>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Button
            v-if="!selectedPlugin.error"
            variant="ghost"
            size="sm"
            :class="headerButtonClass"
            :disabled="togglingId === selectedPlugin.id"
            @click="requestToggle(selectedPlugin)"
          >{{ selectedPlugin.enabled ? '停用' : '启用' }}</Button>
          <Button
            v-if="!selectedPlugin.error && selectedPlugin.updateUrl"
            variant="ghost"
            size="sm"
            :class="headerButtonClass"
            :disabled="updateCheckState === 'checking'"
            @click="checkUpdate"
          >{{ updateCheckState === 'checking' ? '检查中...' : '检查更新' }}</Button>
          <Button
            v-if="!selectedPlugin.error"
            variant="ghost"
            size="sm"
            :class="dangerButtonClass"
            :disabled="uninstalling"
            :title="selectedPlugin.system ? '系统插件不可卸载' : '删除插件目录与设置'"
            @click="requestUninstall(selectedPlugin)"
          >{{ uninstalling ? '卸载中...' : selectedPlugin.system ? '卸载（系统插件）' : '卸载' }}</Button>
          <Button variant="ghost" size="sm" :class="headerButtonClass" @click="refresh">
            刷新
          </Button>
        </div>
      </header>

      <!-- 更新检查结果 -->
      <Card v-if="updateInfo && updateInfo.hasUpdate" :class="panelClass">
        <CardContent class="flex flex-wrap items-center justify-between gap-2 px-3">
          <p class="text-[13px] text-[#475569] dark:text-[#c8c8c8]">
            发现新版本：<span class="font-mono">{{ updateInfo.currentVersion || '?' }}</span> →
            <span class="font-mono text-[#1d4ed8] dark:text-[#93c5fd]">{{ updateInfo.remoteVersion }}</span>
          </p>
          <Button
            variant="ghost"
            size="sm"
            :class="headerButtonClass"
            :disabled="updating"
            @click="updateConfirmVisible = true"
          >{{ updating ? '更新中...' : '更新到新版本' }}</Button>
        </CardContent>
      </Card>

      <!-- 加载失败详情 -->
      <Card v-if="selectedPlugin.error" :class="panelClass">
        <CardContent class="space-y-2 px-3">
          <p class="text-[13px] text-[#dc2626] dark:text-[#fca5a5]">{{ selectedPlugin.error }}</p>
          <p class="break-all font-mono text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">{{ selectedPlugin.dir }}</p>
          <p class="text-[12.5px] text-[#64748b] dark:text-[#a3a3a3]">修正 plugin.json 后刷新即可重新载入。</p>
        </CardContent>
      </Card>

      <template v-else>
        <!-- 元信息 -->
        <Card :class="panelClass">
          <CardContent class="space-y-3 px-3">
            <p class="text-[13px] leading-6 text-[#475569] dark:text-[#c8c8c8]">{{ selectedPlugin.description }}</p>
            <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <div class="space-y-1">
                <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">插件 ID</p>
                <p :class="valueClass" class="break-all font-mono">{{ selectedPlugin.id }}</p>
              </div>
              <div class="space-y-1">
                <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">版本 / 作者</p>
                <p :class="valueClass">
                  {{ selectedPlugin.version || '—' }}{{ selectedPlugin.author ? ` · ${selectedPlugin.author}` : '' }}
                  <span v-if="selectedPlugin.system" class="ml-1 text-[11px] text-[#94a3b8]">（系统内置）</span>
                </p>
              </div>
              <div class="space-y-1">
                <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">安装位置</p>
                <p :class="valueClass" class="break-all font-mono text-[12px]">{{ selectedPlugin.dir }}</p>
              </div>
              <div class="space-y-1">
                <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">权限声明</p>
                <p :class="valueClass" class="font-mono text-[12px]">
                  {{ selectedPlugin.permissions.length > 0 ? selectedPlugin.permissions.join('、') : '无（零权限沙箱）' }}
                </p>
              </div>
            </div>
          </CardContent>
        </Card>

        <!-- 提供给 AI 的工具 -->
        <Card v-if="selectedPlugin.tools.length > 0" :class="panelClass">
          <CardContent class="space-y-2 px-3">
            <p class="text-[13px] font-medium text-[#374151] dark:text-[#d7d7d7]">AI 工具（{{ selectedPlugin.tools.length }}）</p>
            <div
              v-for="tool in selectedPlugin.tools"
              :key="tool.name"
              class="rounded-md border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2 dark:border-[#333] dark:bg-[#262626]"
            >
              <p class="font-mono text-[12.5px] text-[#1d4ed8] dark:text-[#93c5fd]">{{ tool.name }}</p>
              <p class="mt-0.5 text-[12.5px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">{{ tool.description }}</p>
            </div>
            <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">
              启用后这些工具与内置工具进入同一工具池，可被 AI 直接调用，也可在智能体套件中勾选。
            </p>
          </CardContent>
        </Card>

        <!-- 贡献的斜杠命令 -->
        <Card v-if="selectedPlugin.commands.length > 0" :class="panelClass">
          <CardContent class="space-y-2 px-3">
            <p class="text-[13px] font-medium text-[#374151] dark:text-[#d7d7d7]">斜杠命令（{{ selectedPlugin.commands.length }}）</p>
            <div
              v-for="command in selectedPlugin.commands"
              :key="command.name"
              class="rounded-md border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2 dark:border-[#333] dark:bg-[#262626]"
            >
              <p class="font-mono text-[12.5px] text-emerald-700 dark:text-emerald-400">/{{ command.name }}</p>
              <p class="mt-0.5 text-[12.5px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">{{ command.description || command.title }}</p>
            </div>
            <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">
              启用后在输入框输入 / 即出现在候选列表；选中后模板展开为消息发送给 AI（支持 {workspace} / {date} 占位符）。
            </p>
          </CardContent>
        </Card>

        <!-- 贡献的提示词片段 -->
        <Card v-if="selectedPlugin.promptSection" :class="panelClass">
          <CardContent class="space-y-2 px-3">
            <p class="text-[13px] font-medium text-[#374151] dark:text-[#d7d7d7]">
              提示词片段（锚点：{{ selectedPlugin.promptSection.placement }}）
            </p>
            <pre class="whitespace-pre-wrap rounded-md border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2 text-[12px] leading-5 text-[#475569] dark:border-[#333] dark:bg-[#262626] dark:text-[#c8c8c8]">{{ selectedPlugin.promptSection.content }}</pre>
            <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">
              启用后拼接进系统提示词（after-tools=主提示词后 / before-memory=记忆快照后 / end=末尾），引导 AI 遵守插件注入的领域规范。
            </p>
          </CardContent>
        </Card>

        <!-- 插件自带配置页（iframe 沙箱） -->
        <Card v-if="selectedPlugin.enabled && selectedPlugin.settingsTab" :class="panelClass">
          <CardContent class="space-y-2 px-3">
            <p class="text-[13px] font-medium text-[#374151] dark:text-[#d7d7d7]">{{ selectedPlugin.settingsTab.title }}</p>
            <p class="text-[12px] text-[#94a3b8] dark:text-[#8b8b8b]">
              此区域由插件提供，运行在隔离沙箱中，仅能通过受控桥读写本插件配置。
            </p>
            <PluginPanel
              :plugin-id="selectedPlugin.id"
              :plugin-name="selectedPlugin.name"
              :view="selectedPlugin.settingsTab.view"
            />
          </CardContent>
        </Card>
        <Card v-else-if="selectedPlugin.enabled" :class="panelClass">
          <CardContent class="px-3 text-[12.5px] text-[#94a3b8] dark:text-[#8b8b8b]">
            该插件未提供设置页（manifest 中无 settingsTab 声明）。
          </CardContent>
        </Card>
      </template>
    </template>

    <ConfirmDialog
      v-model="confirmVisible"
      :title="`启用「${confirmTarget?.name ?? ''}」？`"
      confirm-text="信任并启用"
      cancel-text="取消"
      @confirm="onConfirmEnable"
    >
      <div class="mt-1 rounded-lg bg-[#fffbeb] p-3 text-[12.5px] leading-relaxed text-[#92400e] dark:bg-[#422006]/40 dark:text-[#fbbf24]">
        该插件申请以下权限，启用后其沙箱代码将获得对应能力：
        <ul class="mt-1.5 space-y-1 font-mono text-[12px]">
          <li v-for="permission in confirmTarget?.permissions ?? []" :key="permission">· {{ permission }}</li>
        </ul>
      </div>
    </ConfirmDialog>

    <ConfirmDialog
      v-model="uninstallVisible"
      :title="`卸载「${uninstallTarget?.name ?? ''}」？`"
      confirm-text="卸载并删除"
      cancel-text="取消"
      @confirm="onConfirmUninstall"
    >
      <div class="mt-1 rounded-lg bg-[#fef2f2] p-3 text-[12.5px] leading-relaxed text-[#991b1b] dark:bg-[#450a0a]/40 dark:text-[#fca5a5]">
        将删除以下内容（不可恢复）：
        <p class="mt-1.5 break-all font-mono text-[12px]">· 插件目录：{{ uninstallTarget?.dir }}</p>
        <p class="mt-1 break-all font-mono text-[12px]">· 插件设置（plugins/.settings/{{ uninstallTarget?.id }}.json）</p>
      </div>
    </ConfirmDialog>

    <ConfirmDialog
      v-model="updateConfirmVisible"
      :title="`更新「${selectedPlugin?.name ?? ''}」？`"
      confirm-text="下载并更新"
      cancel-text="取消"
      @confirm="onConfirmUpdate"
    >
      <div class="mt-1 rounded-lg bg-[#eff6ff] p-3 text-[12.5px] leading-relaxed text-[#1e40af] dark:bg-[#1e293b]/60 dark:text-[#93c5fd]">
        将从更新源下载新版本 zip 并覆盖安装（插件设置保留）：
        <p class="mt-1.5 font-mono text-[12px]">· {{ selectedPlugin?.updateUrl }}</p>
        <p class="mt-1.5 font-mono text-[12px]">· {{ updateInfo?.currentVersion || '?' }} → {{ updateInfo?.remoteVersion }}</p>
      </div>
    </ConfirmDialog>
  </div>
</template>
