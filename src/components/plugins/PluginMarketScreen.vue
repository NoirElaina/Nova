<script setup lang="ts">
// 插件市场/管理页（主界面视图，替代原 Agent 市场占位页）。
// 两级视图：grid = 插件卡片网格；detail = 单插件详情（元信息 + 启停 + iframe 配置页）。
// 插件由用户放入应用数据目录 plugins/ 下，这里负责发现、启停（带权限确认）与配置。
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import ConfirmDialog from "@/components/ui/confirm-dialog/ConfirmDialog.vue";
import PluginPanel from "@/components/plugins/PluginPanel.vue";

type MainView = "chat" | "hooks" | "agent" | "plugins" | "schedule" | "settings";

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
}>();

type PluginTool = { name: string; description: string };

type PluginInfo = {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  permissions: string[];
  tools: PluginTool[];
  settingsTab: { title: string; icon: string; view: string } | null;
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

// 启用带权限声明的插件时弹一次确认。
const confirmTarget = ref<PluginInfo | null>(null);
const confirmVisible = ref(false);

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

const backToChat = () => emit("change-main-view", "chat");

onMounted(refresh);

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
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
            安装在 plugins 目录下的扩展：为 AI 增加工具、自带配置界面。已启用 {{ enabledCount }} / {{ plugins.length }}。
          </p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Button variant="ghost" size="sm" :class="headerButtonClass" @click="backToChat">
            返回聊天
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
          <p>暂无插件。将插件目录（含 plugin.json 与 main.js）放入应用数据目录的 plugins 子目录后刷新。</p>
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

        <!-- 帮助卡片 -->
        <div class="flex min-h-[120px] flex-col justify-center gap-1.5 rounded-xl border border-dashed border-[#c7d4e8] bg-white/40 p-4 text-[12px] leading-5 text-[#64748b] dark:border-[#3f3f3f] dark:bg-transparent dark:text-[#a3a3a3]">
          <p class="text-[13px] font-medium text-[#475569] dark:text-[#d7d7d7]">编写插件</p>
          <p>plugin.json 声明 id、工具 schema、权限与设置页；main.js 用 nova.tool(name, handler) 注册实现。</p>
          <p>插件界面放 ui/ 目录并在 manifest 的 settingsTab 声明，即可像内置页面一样嵌入配置详情。</p>
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
        <div class="flex items-center gap-2">
          <Button
            v-if="!selectedPlugin.error"
            variant="ghost"
            size="sm"
            :class="headerButtonClass"
            :disabled="togglingId === selectedPlugin.id"
            @click="requestToggle(selectedPlugin)"
          >{{ selectedPlugin.enabled ? '停用' : '启用' }}</Button>
          <Button variant="ghost" size="sm" :class="headerButtonClass" @click="refresh">
            刷新
          </Button>
        </div>
      </header>

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
                <p :class="valueClass">{{ selectedPlugin.version || '—' }}{{ selectedPlugin.author ? ` · ${selectedPlugin.author}` : '' }}</p>
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
  </div>
</template>
