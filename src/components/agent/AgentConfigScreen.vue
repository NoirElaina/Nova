<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { emitToast } from "@/lib/toast";

type MainView = "chat" | "hooks" | "agent";

type AgentBundle = {
  id: string;
  name: string;
  description: string;
  prompt: string;
  enabledTools: string[] | null;
  enabledSkills: string[] | null;
  enabledMcpServers: string[] | null;
  createdAt: number;
  updatedAt: number;
};

type ConfigurableTool = {
  name: string;
  description: string;
  readOnly: boolean;
  alwaysOn: boolean;
};

type SkillItem = { name: string; description: string; path: string };
type McpServerStatus = { name: string; status: string; type: string; enabled: boolean };

/** 智能体资料文件条目（agents/<id>/files/ 顶层）。 */
type AgentFileEntry = { name: string; sizeBytes: number; isDir: boolean };

/** 私有 MCP server 条目（agents/<id>/mcp.json）。 */
type McpServerConfigJson = Record<string, unknown>;
type AgentMcpEntry = { name: string; enabled: boolean; config: McpServerConfigJson };

const props = defineProps<{
  /** 当前对话 id：用于标记哪个对话挂载了哪个智能体。 */
  conversationId: string | null;
}>();

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
  /** 点击「启用」：请求新建一个对话并挂载该智能体。 */
  (e: "launch-agent", bundleId: string): void;
}>();

const loading = ref(false);
const saving = ref(false);
const creating = ref(false);
const activating = ref(false);
const showCreatePanel = ref(false);
const showDeletePanel = ref(false);
const newBundleName = ref("new-agent");
const createInputRef = ref<HTMLInputElement | null>(null);

/** 页面两级视图：grid = 卡片列表；editor = 单个套件的配置页。 */
const view = ref<"grid" | "editor">("grid");

/** 配置页弹窗：空串 = 关闭。各弹窗内编辑直接绑定 draft/selection，改动计入 hasChanges。 */
type EditorModal =
  | ""
  | "tools"
  | "skills"
  | "mcp"
  | "privateSkills"
  | "files"
  | "privateMcp";
const activeModal = ref<EditorModal>("");

const bundles = ref<AgentBundle[]>([]);
const conversationAgentId = ref<string | null>(null);
const selectedId = ref("");

const configurableTools = ref<ConfigurableTool[]>([]);
const skills = ref<SkillItem[]>([]);
const mcpServers = ref<McpServerStatus[]>([]);

// 编辑态（选中 bundle 的可变副本）
const draft = ref<AgentBundle | null>(null);
const original = ref<string>("");

// 每类能力双模式：false = 全部（跟随全局，存 null）；true = 自定义勾选清单（勾=添加，取消=移除）
const useCustomTools = ref(false);
const toolSelection = ref<Set<string>>(new Set());
const useCustomSkills = ref(false);
const skillSelection = ref<Set<string>>(new Set());
const useCustomMcp = ref(false);
const mcpSelection = ref<Set<string>>(new Set());

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const fieldClass =
  "border-[#d8dee8] bg-white text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const primaryButtonClass =
  "h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] focus-visible:ring-[#111827]/20 dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white";
const modeTabActiveClass =
  "bg-[#111827] text-white dark:bg-[#ededed] dark:text-[#111]";
const modeTabIdleClass =
  "bg-white text-[#475569] hover:bg-[#f4f7fb] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";

const selectedBundle = computed(() => bundles.value.find((b) => b.id === selectedId.value) ?? null);
const hasChanges = computed(() => !!draft.value && serializeDraft() !== original.value);
const hasConversation = computed(() => !!props.conversationId?.trim());
const isConversationAgent = computed(
  () => !!draft.value && draft.value.id === conversationAgentId.value,
);

function serializeDraft(): string {
  if (!draft.value) return "";
  return JSON.stringify({
    ...draft.value,
    enabledTools: useCustomTools.value ? [...toolSelection.value].sort() : null,
    enabledSkills: useCustomSkills.value ? [...skillSelection.value].sort() : null,
    enabledMcpServers: useCustomMcp.value ? [...mcpSelection.value].sort() : null,
  });
}

function applyBundleToEditor(bundle: AgentBundle) {
  draft.value = { ...bundle };
  useCustomTools.value = bundle.enabledTools !== null;
  toolSelection.value = new Set(bundle.enabledTools ?? []);
  useCustomSkills.value = bundle.enabledSkills !== null;
  skillSelection.value = new Set(bundle.enabledSkills ?? []);
  useCustomMcp.value = bundle.enabledMcpServers !== null;
  mcpSelection.value = new Set(bundle.enabledMcpServers ?? []);
  original.value = serializeDraft();
}

const formatUpdatedAt = (unixSeconds: number) => {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) return "--";
  return new Date(unixSeconds * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
};

async function loadCatalog() {
  const [tools, skillList, servers] = await Promise.all([
    invoke<ConfigurableTool[]>("list_configurable_tools").catch(() => []),
    invoke<SkillItem[]>("list_skills").catch(() => []),
    invoke<McpServerStatus[]>("get_mcp_server_statuses").catch(() => []),
  ]);
  configurableTools.value = tools ?? [];
  skills.value = skillList ?? [];
  mcpServers.value = servers ?? [];
}

async function loadConversationAgent() {
  if (!hasConversation.value) {
    conversationAgentId.value = null;
    return;
  }
  try {
    const bundle = await invoke<AgentBundle | null>("get_conversation_agent", {
      conversationId: props.conversationId,
    });
    conversationAgentId.value = bundle?.id ?? null;
  } catch {
    conversationAgentId.value = null;
  }
}

async function loadBundles() {
  loading.value = true;
  try {
    const items = await invoke<AgentBundle[]>("list_agent_bundles");
    bundles.value = items ?? [];

    // 编辑器打开中：选中项已不存在（如被删除）则退回卡片列表。
    if (view.value === "editor" && !bundles.value.some((b) => b.id === selectedId.value)) {
      selectedId.value = "";
      draft.value = null;
      original.value = "";
      view.value = "grid";
    }
  } catch (err) {
    console.error("Failed to load agent bundles:", err);
  } finally {
    loading.value = false;
  }
}

/** 点击卡片：进入该套件的配置页。 */
function openBundleEditor(id: string) {
  const bundle = bundles.value.find((b) => b.id === id);
  if (!bundle) return;
  selectedId.value = id;
  applyBundleToEditor(bundle);
  view.value = "editor";
  void loadAgentResources(id);
}

// ---------------- 智能体目录资源：私有技能 / 资料文件 / 私有 MCP ----------------

const privateSkills = ref<SkillItem[]>([]);
const agentFiles = ref<AgentFileEntry[]>([]);
const agentMcp = ref<AgentMcpEntry[]>([]);
const resourcesLoading = ref(false);

/** 私有 MCP 表单（stdio / streamable_http 两种常用类型）。 */
const showMcpForm = ref(false);
const mcpForm = ref<{
  oldName: string | null;
  name: string;
  kind: "stdio" | "streamable_http";
  command: string;
  args: string;
  url: string;
  enabled: boolean;
}>({ oldName: null, name: "", kind: "stdio", command: "", args: "", url: "", enabled: true });

async function loadAgentResources(bundleId: string) {
  resourcesLoading.value = true;
  try {
    const [skillsRes, filesRes, mcpRes] = await Promise.all([
      invoke<SkillItem[]>("list_agent_private_skills", { bundleId }).catch(() => []),
      invoke<AgentFileEntry[]>("list_agent_files", { bundleId }).catch(() => []),
      invoke<AgentMcpEntry[]>("list_agent_mcp_servers", { bundleId }).catch(() => []),
    ]);
    privateSkills.value = skillsRes ?? [];
    agentFiles.value = filesRes ?? [];
    agentMcp.value = mcpRes ?? [];
  } finally {
    resourcesLoading.value = false;
  }
}

async function deletePrivateSkill(skill: SkillItem) {
  if (!selectedId.value || !confirm(`确定删除私有技能「${skill.name}」？此操作不可恢复。`)) return;
  try {
    await invoke("delete_skill", { path: skill.path });
    await loadAgentResources(selectedId.value);
  } catch (err) {
    console.error("Failed to delete agent private skill:", err);
  }
}

async function importAgentFile() {
  if (!selectedId.value) return;
  const picked = await openFileDialog({ multiple: false, directory: false });
  const src = Array.isArray(picked) ? picked[0] : picked;
  if (!src) return;
  try {
    await invoke("import_agent_file", { bundleId: selectedId.value, srcPath: src });
    await loadAgentResources(selectedId.value);
    emitToast({ variant: "success", source: "agent-config", message: "资料文件已导入。" });
  } catch (err) {
    console.error("Failed to import agent file:", err);
  }
}

async function deleteAgentFile(name: string) {
  if (!selectedId.value || !confirm(`确定删除资料「${name}」？`)) return;
  try {
    await invoke("delete_agent_file", { bundleId: selectedId.value, name });
    await loadAgentResources(selectedId.value);
  } catch (err) {
    console.error("Failed to delete agent file:", err);
  }
}

async function revealAgentDir() {
  if (!selectedId.value) return;
  try {
    await invoke("reveal_agent_dir", { bundleId: selectedId.value });
  } catch (err) {
    console.error("Failed to reveal agent dir:", err);
  }
}

function openMcpForm(entry: AgentMcpEntry | null) {
  if (entry) {
    const cfg = entry.config ?? {};
    const isStdio = cfg.type === "stdio";
    const args = Array.isArray(cfg.args) ? (cfg.args as string[]).join(" ") : "";
    const command = typeof cfg.command === "string" ? cfg.command : "";
    const url = typeof cfg.url === "string" ? cfg.url : "";
    mcpForm.value = {
      oldName: entry.name,
      name: entry.name,
      kind: isStdio ? "stdio" : "streamable_http",
      command,
      args,
      url,
      enabled: entry.enabled,
    };
  } else {
    mcpForm.value = {
      oldName: null,
      name: "",
      kind: "stdio",
      command: "",
      args: "",
      url: "",
      enabled: true,
    };
  }
  showMcpForm.value = true;
}

async function saveAgentMcp() {
  if (!selectedId.value) return;
  const form = mcpForm.value;
  const name = form.name.trim();
  if (!name) {
    emitToast({ variant: "error", source: "agent-config", message: "请输入 server 名称。" });
    return;
  }
  let config: McpServerConfigJson;
  if (form.kind === "stdio") {
    if (!form.command.trim()) {
      emitToast({ variant: "error", source: "agent-config", message: "请输入启动命令。" });
      return;
    }
    config = {
      type: "stdio",
      command: form.command.trim(),
      // args 按空白切分（与常见 MCP 配置习惯一致）。
      args: form.args.trim() ? form.args.trim().split(/\s+/) : [],
    };
  } else {
    if (!form.url.trim()) {
      emitToast({ variant: "error", source: "agent-config", message: "请输入 URL。" });
      return;
    }
    config = { type: "streamable_http", url: form.url.trim() };
  }
  try {
    await invoke("upsert_agent_mcp_server", {
      bundleId: selectedId.value,
      oldName: form.oldName,
      newName: name,
      config,
      enabled: form.enabled,
    });
    showMcpForm.value = false;
    await loadAgentResources(selectedId.value);
    emitToast({ variant: "success", source: "agent-config", message: "私有 MCP 已保存并连接。" });
  } catch (err) {
    console.error("Failed to save agent mcp server:", err);
  }
}

async function deleteAgentMcp(entry: AgentMcpEntry) {
  if (!selectedId.value || !confirm(`确定删除私有 MCP「${entry.name}」？`)) return;
  try {
    await invoke("upsert_agent_mcp_server", {
      bundleId: selectedId.value,
      oldName: entry.name,
      newName: null,
      config: null,
      enabled: false,
    });
    await loadAgentResources(selectedId.value);
  } catch (err) {
    console.error("Failed to delete agent mcp server:", err);
  }
}

const formatFileSize = (bytes: number) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
};

// ---------------- 配置页大类摘要（按钮卡片上显示的当前值） ----------------

const toolsSummary = computed(() =>
  useCustomTools.value
    ? `自定义 ${toolSelection.value.size} / ${configurableTools.value.length} 项`
    : `全部 ${configurableTools.value.length} 项`,
);
const skillsSummary = computed(() =>
  useCustomSkills.value
    ? `自定义 ${skillSelection.value.size} / ${skills.value.length} 项`
    : `全部 ${skills.value.length} 项`,
);
const mcpSummary = computed(() =>
  useCustomMcp.value
    ? `自定义 ${mcpSelection.value.size} / ${mcpServers.value.length} 项`
    : `全部 ${mcpServers.value.length} 项`,
);
const privateSkillsSummary = computed(() => `${privateSkills.value.length} 个`);
const filesSummary = computed(() => `${agentFiles.value.length} 个文件`);
const privateMcpSummary = computed(() => `${agentMcp.value.length} 个 server`);

/** 打开配置弹窗；专属内容类同时刷新对应资源列表。 */
function openModal(modal: EditorModal) {
  activeModal.value = modal;
  if ((modal === "privateSkills" || modal === "files" || modal === "privateMcp") && selectedId.value) {
    void loadAgentResources(selectedId.value);
  }
}

/** 配置页返回卡片列表（有未保存改动时拦截）。 */
function backToGrid() {
  if (hasChanges.value) {
    emitToast({
      variant: "error",
      source: "agent-config",
      message: "当前有未保存改动，请先保存或放弃后再返回。",
    });
    return;
  }
  view.value = "grid";
}

type SelectionKey = "tools" | "skills" | "mcp";

function toggleSelection(key: SelectionKey, name: string) {
  const target =
    key === "tools" ? toolSelection : key === "skills" ? skillSelection : mcpSelection;
  const next = new Set(target.value);
  if (next.has(name)) {
    next.delete(name);
  } else {
    next.add(name);
  }
  target.value = next;
}

/** 从「全部」切到「自定义」：以当前全量勾选为起点，用户再自由加减。 */
function enableCustomMode(key: SelectionKey) {
  if (key === "tools") {
    if (toolSelection.value.size === 0) {
      toolSelection.value = new Set(configurableTools.value.map((t) => t.name));
    }
    useCustomTools.value = true;
    return;
  }
  if (key === "skills") {
    if (skillSelection.value.size === 0) {
      skillSelection.value = new Set(skills.value.map((s) => s.name));
    }
    useCustomSkills.value = true;
    return;
  }
  if (mcpSelection.value.size === 0) {
    mcpSelection.value = new Set(mcpServers.value.map((s) => s.name));
  }
  useCustomMcp.value = true;
}

function setAllSelection(key: SelectionKey, all: boolean) {
  if (key === "tools") {
    toolSelection.value = all ? new Set(configurableTools.value.map((t) => t.name)) : new Set();
    return;
  }
  if (key === "skills") {
    skillSelection.value = all ? new Set(skills.value.map((s) => s.name)) : new Set();
    return;
  }
  mcpSelection.value = all ? new Set(mcpServers.value.map((s) => s.name)) : new Set();
}

function openCreateDialog() {
  newBundleName.value = "";
  showCreatePanel.value = true;
  // 弹窗渲染完成后聚焦输入框。
  void nextTick(() => {
    createInputRef.value?.focus();
  });
}

async function createBundle() {
  if (creating.value) return;
  const name = newBundleName.value.trim();
  if (!name) {
    emitToast({ variant: "error", source: "agent-config", message: "请输入智能体名称。" });
    return;
  }
  creating.value = true;
  try {
    const created = await invoke<AgentBundle>("create_agent_bundle", { name });
    showCreatePanel.value = false;
    await loadBundles();
    // 创建完成后直接进入配置页。
    if (created?.id) openBundleEditor(created.id);
    emitToast({ variant: "success", source: "agent-config", message: "已创建智能体套件。" });
  } catch (err) {
    console.error("Failed to create agent bundle:", err);
  } finally {
    creating.value = false;
  }
}

async function saveBundle() {
  if (!draft.value || saving.value) return;
  saving.value = true;
  try {
    const bundle: AgentBundle = {
      ...draft.value,
      enabledTools: useCustomTools.value ? [...toolSelection.value] : null,
      enabledSkills: useCustomSkills.value ? [...skillSelection.value] : null,
      enabledMcpServers: useCustomMcp.value ? [...mcpSelection.value] : null,
    };
    const saved = await invoke<AgentBundle>("save_agent_bundle", { bundle });
    await loadBundles();
    // 用服务端返回值重置草稿基线，清除“未保存改动”状态。
    if (saved) applyBundleToEditor(saved);
    emitToast({ variant: "success", source: "agent-config", message: "智能体套件已保存。" });
  } catch (err) {
    console.error("Failed to save agent bundle:", err);
  } finally {
    saving.value = false;
  }
}

/** 点击「启用」：新开一个对话并挂载该智能体（由 App.vue 负责建会话+挂载+跳转）。 */
function launchAgent() {
  if (!draft.value || activating.value) return;
  if (hasChanges.value) {
    emitToast({
      variant: "error",
      source: "agent-config",
      message: "当前有未保存改动，请先保存或放弃后再启用。",
    });
    return;
  }
  activating.value = true;
  emit("launch-agent", draft.value.id);
  // App.vue 跳转回聊天页会卸载本组件，这里兜底复位（未跳转时按钮可再点）。
  window.setTimeout(() => {
    activating.value = false;
  }, 600);
}

/** 卡片上的删除：设置目标 id 后弹出确认（复用删除弹窗与 deleteBundle）。 */
function openDeleteFromGrid(bundle: AgentBundle) {
  selectedId.value = bundle.id;
  showDeletePanel.value = true;
}

async function deleteBundle() {
  if (!selectedId.value) return;
  try {
    await invoke("delete_agent_bundle", { bundleId: selectedId.value });
    showDeletePanel.value = false;
    if (conversationAgentId.value === selectedId.value) {
      conversationAgentId.value = null;
      window.dispatchEvent(new CustomEvent("agent-bundle-changed"));
    }
    emitToast({ variant: "success", source: "agent-config", message: "已删除智能体套件。" });
    // 删除后回到卡片列表。
    selectedId.value = "";
    draft.value = null;
    original.value = "";
    view.value = "grid";
    await loadBundles();
  } catch (err) {
    console.error("Failed to delete agent bundle:", err);
  }
}

function discardChanges() {
  const bundle = selectedBundle.value;
  if (bundle) applyBundleToEditor(bundle);
}

onMounted(async () => {
  await Promise.all([loadBundles(), loadCatalog(), loadConversationAgent()]);
});
</script>

<template>
  <div :class="pageClass">
    <header v-if="view === 'grid'" class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">智能体套件</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
          每个套件 = 提示词 + 工具/技能/MCP 装备清单。点击卡片进入配置，启用后新开对话使用该智能体。
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button size="sm" :class="primaryButtonClass" :disabled="creating" @click="openCreateDialog">
          新建套件
        </Button>
        <Button variant="outline" size="sm" :class="headerButtonClass" @click="emit('change-main-view', 'chat')">
          返回聊天
        </Button>
      </div>
    </header>

    <!-- 新建套件弹窗：带名称输入框，Enter 直接创建 -->
    <ConfirmDialog
      v-model="showCreatePanel"
      title="新建智能体套件"
      description="新建套件默认满配全部能力，之后可自由加减。"
      confirm-text="创建"
      cancel-text="取消"
      :busy="creating"
      @confirm="createBundle"
    >

      <input
        ref="createInputRef"
        v-model="newBundleName"
        class="h-9 w-full rounded-md border border-[#d8dee8] bg-white px-3 text-[14px] text-[#111827] outline-none transition-colors placeholder:text-[#a3a3a3] focus:border-[#2563eb] dark:border-[#3a3a3a] dark:bg-[#202020] dark:text-[#ededed] dark:placeholder:text-[#666] dark:focus:border-[#60a5fa]"
        placeholder="例如: code-reviewer"
        :disabled="creating"
        @keydown.enter.prevent="createBundle"
      >
    </ConfirmDialog>

    <!-- 删除确认弹窗 -->
    <ConfirmDialog
      v-model="showDeletePanel"
      title="删除智能体套件"
      :description="`将删除「${selectedBundle?.name ?? ''}」整套配置，所有挂载它的对话自动回到默认 Nova。不可恢复。`"
      confirm-text="删除"
      cancel-text="取消"
      destructive
      @confirm="deleteBundle"
    />

    <!-- 私有 MCP 编辑弹窗 -->
    <ConfirmDialog
      v-model="showMcpForm"
      width-class="max-w-[560px]"
      :title="mcpForm.oldName ? `编辑私有 MCP「${mcpForm.oldName}」` : '添加私有 MCP'"
      :description="mcpForm.oldName ? '修改配置后保存会重连该 server。' : '保存后会立即连接该 server（stdio 启动可能需要数十秒）。'"
      confirm-text="保存"
      cancel-text="取消"
      @confirm="saveAgentMcp"
    >
      <div class="space-y-2.5">
        <div class="grid grid-cols-[110px_minmax(0,1fr)] items-center gap-2">
          <span class="text-right text-[12.5px] text-[#475569] dark:text-[#a3a3a3]">名称</span>
          <Input v-model="mcpForm.name" :class="fieldClass" placeholder="例如: my-server" />
        </div>
        <div class="grid grid-cols-[110px_minmax(0,1fr)] items-center gap-2">
          <span class="text-right text-[12.5px] text-[#475569] dark:text-[#a3a3a3]">类型</span>
          <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
            <button
              type="button"
              class="px-3 py-1 transition-colors"
              :class="mcpForm.kind === 'stdio' ? modeTabActiveClass : modeTabIdleClass"
              @click="mcpForm.kind = 'stdio'"
            >stdio</button>
            <button
              type="button"
              class="border-l border-[#d8dee8] px-3 py-1 transition-colors dark:border-[#3a3a3a]"
              :class="mcpForm.kind === 'streamable_http' ? modeTabActiveClass : modeTabIdleClass"
              @click="mcpForm.kind = 'streamable_http'"
            >HTTP</button>
          </div>
        </div>
        <template v-if="mcpForm.kind === 'stdio'">
          <div class="grid grid-cols-[110px_minmax(0,1fr)] items-center gap-2">
            <span class="text-right text-[12.5px] text-[#475569] dark:text-[#a3a3a3]">命令</span>
            <Input v-model="mcpForm.command" :class="fieldClass" placeholder="例如: npx" />
          </div>
          <div class="grid grid-cols-[110px_minmax(0,1fr)] items-center gap-2">
            <span class="text-right text-[12.5px] text-[#475569] dark:text-[#a3a3a3]">参数</span>
            <Input v-model="mcpForm.args" :class="fieldClass" placeholder="例如: -y @modelcontextprotocol/server-filesystem /path" />
          </div>
        </template>
        <div v-else class="grid grid-cols-[110px_minmax(0,1fr)] items-center gap-2">
          <span class="text-right text-[12.5px] text-[#475569] dark:text-[#a3a3a3]">URL</span>
          <Input v-model="mcpForm.url" :class="fieldClass" placeholder="https://example.com/mcp" />
        </div>
        <label class="flex cursor-pointer items-center gap-2 pl-[122px] text-[12.5px] text-[#475569] dark:text-[#a3a3a3]">
          <input type="checkbox" class="accent-[#2563eb]" v-model="mcpForm.enabled" />
          保存后立即启用
        </label>
        <div class="text-[11px] text-[#98a2b3] dark:text-[#9d9589]">
          需要高级字段（env/headers）时可直接编辑 agents/&lt;id&gt;/mcp.json 文件。
        </div>
      </div>
    </ConfirmDialog>

    <!-- ============ 配置页大类弹窗 ============ -->

    <!-- 内置工具 -->
    <ConfirmDialog
      :model-value="activeModal === 'tools'"
      width-class="max-w-[720px]"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      title="内置工具"
      description="「全部」自动包含后续新增的工具；「自定义」仅勾选项可用。"
      confirm-text="完成"
      cancel-text="关闭"
      @confirm="activeModal = ''"
    >
      <div class="max-h-[52vh] space-y-2 overflow-y-auto pr-1 custom-scrollbar">
        <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
          <button
            type="button"
            class="flex-1 px-3 py-1.5 transition-colors"
            :class="!useCustomTools ? modeTabActiveClass : modeTabIdleClass"
            @click="useCustomTools = false"
          >全部（{{ configurableTools.length }}）</button>
          <button
            type="button"
            class="flex-1 border-l border-[#d8dee8] px-3 py-1.5 transition-colors dark:border-[#3a3a3a]"
            :class="useCustomTools ? modeTabActiveClass : modeTabIdleClass"
            @click="enableCustomMode('tools')"
          >自定义（{{ toolSelection.size }}）</button>
        </div>
        <template v-if="useCustomTools">
          <div class="flex items-center justify-between">
            <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">勾选 = 添加，取消 = 移除</span>
            <div class="flex gap-1">
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('tools', true)">全选</Button>
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('tools', false)">清空</Button>
            </div>
          </div>
          <div class="grid grid-cols-1 gap-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333] sm:grid-cols-2">
            <label
              v-for="tool in configurableTools"
              :key="tool.name"
              class="flex items-start gap-1.5 rounded px-1 py-0.5 text-[12px] hover:bg-[#f8fafc] dark:hover:bg-[#2a2a2a]"
              :class="tool.alwaysOn ? 'cursor-default opacity-70' : 'cursor-pointer'"
            >
              <input
                type="checkbox"
                class="mt-0.5 accent-[#2563eb]"
                :checked="tool.alwaysOn || toolSelection.has(tool.name)"
                :disabled="tool.alwaysOn"
                @change="toggleSelection('tools', tool.name)"
              />
              <span class="min-w-0">
                <span class="font-mono">{{ tool.name }}</span>
                <span v-if="tool.alwaysOn" class="ml-1 text-[10px] text-[#64748b] dark:text-[#a3a3a3]">（流程必需）</span>
              </span>
            </label>
          </div>
        </template>
      </div>
    </ConfirmDialog>

    <!-- 全局技能 -->
    <ConfirmDialog
      :model-value="activeModal === 'skills'"
      width-class="max-w-[720px]"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      title="全局技能"
      description="从全局技能库中勾选；全局停用的技能始终不可见。"
      confirm-text="完成"
      cancel-text="关闭"
      @confirm="activeModal = ''"
    >
      <div class="max-h-[52vh] space-y-2 overflow-y-auto pr-1 custom-scrollbar">
        <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
          <button
            type="button"
            class="flex-1 px-3 py-1.5 transition-colors"
            :class="!useCustomSkills ? modeTabActiveClass : modeTabIdleClass"
            @click="useCustomSkills = false"
          >全部（{{ skills.length }}）</button>
          <button
            type="button"
            class="flex-1 border-l border-[#d8dee8] px-3 py-1.5 transition-colors dark:border-[#3a3a3a]"
            :class="useCustomSkills ? modeTabActiveClass : modeTabIdleClass"
            @click="enableCustomMode('skills')"
          >自定义（{{ skillSelection.size }}）</button>
        </div>
        <template v-if="useCustomSkills">
          <div class="flex items-center justify-between">
            <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">勾选 = 添加，取消 = 移除</span>
            <div class="flex gap-1">
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('skills', true)">全选</Button>
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('skills', false)">清空</Button>
            </div>
          </div>
          <div class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
            <label
              v-for="skill in skills"
              :key="skill.path"
              class="flex cursor-pointer items-start gap-1.5 rounded px-1 py-0.5 text-[12px] hover:bg-[#f8fafc] dark:hover:bg-[#2a2a2a]"
            >
              <input
                type="checkbox"
                class="mt-0.5 accent-[#2563eb]"
                :checked="skillSelection.has(skill.name)"
                @change="toggleSelection('skills', skill.name)"
              />
              <span class="min-w-0">
                <span class="font-medium">{{ skill.name }}</span>
                <span class="ml-1 text-[#64748b] dark:text-[#a3a3a3]">{{ skill.description }}</span>
              </span>
            </label>
            <div v-if="skills.length === 0" class="px-1 text-[11px] text-[#64748b] dark:text-[#a3a3a3]">
              未发现技能（放置于 app_data/skills/*/SKILL.md）。
            </div>
          </div>
        </template>
      </div>
    </ConfirmDialog>

    <!-- 全局 MCP -->
    <ConfirmDialog
      :model-value="activeModal === 'mcp'"
      width-class="max-w-[720px]"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      title="全局 MCP 服务器"
      description="从全局已配置的 MCP server 中勾选接入。"
      confirm-text="完成"
      cancel-text="关闭"
      @confirm="activeModal = ''"
    >
      <div class="max-h-[52vh] space-y-2 overflow-y-auto pr-1 custom-scrollbar">
        <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
          <button
            type="button"
            class="flex-1 px-3 py-1.5 transition-colors"
            :class="!useCustomMcp ? modeTabActiveClass : modeTabIdleClass"
            @click="useCustomMcp = false"
          >全部（{{ mcpServers.length }}）</button>
          <button
            type="button"
            class="flex-1 border-l border-[#d8dee8] px-3 py-1.5 transition-colors dark:border-[#3a3a3a]"
            :class="useCustomMcp ? modeTabActiveClass : modeTabIdleClass"
            @click="enableCustomMode('mcp')"
          >自定义（{{ mcpSelection.size }}）</button>
        </div>
        <template v-if="useCustomMcp">
          <div class="flex items-center justify-between">
            <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">勾选 = 接入，取消 = 断开</span>
            <div class="flex gap-1">
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('mcp', true)">全选</Button>
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('mcp', false)">清空</Button>
            </div>
          </div>
          <div class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
            <label
              v-for="server in mcpServers"
              :key="server.name"
              class="flex cursor-pointer items-start gap-1.5 rounded px-1 py-0.5 text-[12px] hover:bg-[#f8fafc] dark:hover:bg-[#2a2a2a]"
            >
              <input
                type="checkbox"
                class="mt-0.5 accent-[#2563eb]"
                :checked="mcpSelection.has(server.name)"
                @change="toggleSelection('mcp', server.name)"
              />
              <span class="min-w-0">
                <span class="font-medium">{{ server.name }}</span>
                <span class="ml-1 text-[#64748b] dark:text-[#a3a3a3]">{{ server.status }}</span>
              </span>
            </label>
            <div v-if="mcpServers.length === 0" class="px-1 text-[11px] text-[#64748b] dark:text-[#a3a3a3]">
              暂无已配置的 MCP 服务器。
            </div>
          </div>
        </template>
      </div>
    </ConfirmDialog>

    <!-- 私有技能 -->
    <ConfirmDialog
      :model-value="activeModal === 'privateSkills'"
      width-class="max-w-[720px]"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      title="私有技能"
      description="仅该智能体可见的技能，放在 agents/<id>/skills/<名称>/SKILL.md。"
      confirm-text="完成"
      cancel-text="关闭"
      @confirm="activeModal = ''"
    >
      <div class="max-h-[52vh] space-y-2 overflow-y-auto pr-1 custom-scrollbar">
        <div class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
          <div
            v-for="skill in privateSkills"
            :key="skill.path"
            class="flex items-center justify-between gap-2 rounded px-1 py-0.5 text-[12px] hover:bg-[#f8fafc] dark:hover:bg-[#2a2a2a]"
          >
            <span class="min-w-0 truncate">
              <span class="font-medium">{{ skill.name }}</span>
              <span class="ml-1 text-[#64748b] dark:text-[#a3a3a3]">{{ skill.description }}</span>
            </span>
            <button
              type="button"
              class="shrink-0 text-[11px] text-[#dc2626] hover:underline dark:text-[#fca5a5]"
              @click="deletePrivateSkill(skill)"
            >删除</button>
          </div>
          <div v-if="privateSkills.length === 0" class="px-1 text-[11px] text-[#64748b] dark:text-[#a3a3a3]">
            在 skills/&lt;名称&gt;/SKILL.md 放置技能文件即可。
          </div>
        </div>
        <Button variant="outline" size="sm" :class="headerButtonClass" @click="revealAgentDir">打开目录</Button>
      </div>
    </ConfirmDialog>

    <!-- 资料文件 -->
    <ConfirmDialog
      :model-value="activeModal === 'files'"
      width-class="max-w-[720px]"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      title="资料文件"
      description="该智能体的参考资料，AI 使用时可用 Read/Glob/Grep 按绝对路径访问。"
      confirm-text="完成"
      cancel-text="关闭"
      @confirm="activeModal = ''"
    >
      <div class="max-h-[52vh] space-y-2 overflow-y-auto pr-1 custom-scrollbar">
        <div class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
          <div
            v-for="file in agentFiles"
            :key="file.name"
            class="flex items-center justify-between gap-2 rounded px-1 py-0.5 text-[12px] hover:bg-[#f8fafc] dark:hover:bg-[#2a2a2a]"
          >
            <span class="min-w-0 truncate">
              <span class="font-medium">{{ file.name }}</span>
              <span class="ml-1 text-[#64748b] dark:text-[#a3a3a3]">{{ file.isDir ? "目录" : formatFileSize(file.sizeBytes) }}</span>
            </span>
            <button
              type="button"
              class="shrink-0 text-[11px] text-[#dc2626] hover:underline dark:text-[#fca5a5]"
              @click="deleteAgentFile(file.name)"
            >删除</button>
          </div>
          <div v-if="agentFiles.length === 0" class="px-1 text-[11px] text-[#64748b] dark:text-[#a3a3a3]">
            暂无资料文件。
          </div>
        </div>
        <div class="flex gap-2">
          <Button variant="outline" size="sm" :class="headerButtonClass" @click="importAgentFile">导入文件</Button>
          <Button variant="outline" size="sm" :class="headerButtonClass" @click="revealAgentDir">打开目录</Button>
        </div>
      </div>
    </ConfirmDialog>

    <!-- 私有 MCP -->
    <ConfirmDialog
      :model-value="activeModal === 'privateMcp'"
      width-class="max-w-[720px]"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      title="私有 MCP 服务器"
      description="该智能体专属的 MCP server（agents/<id>/mcp.json），仅挂载它的会话可见。"
      confirm-text="完成"
      cancel-text="关闭"
      @confirm="activeModal = ''"
    >
      <div class="max-h-[52vh] space-y-2 overflow-y-auto pr-1 custom-scrollbar">
        <div class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
          <div
            v-for="entry in agentMcp"
            :key="entry.name"
            class="flex items-center justify-between gap-2 rounded px-1 py-0.5 text-[12px] hover:bg-[#f8fafc] dark:hover:bg-[#2a2a2a]"
          >
            <span class="min-w-0 truncate">
              <span class="font-medium">{{ entry.name }}</span>
              <span class="ml-1 text-[#64748b] dark:text-[#a3a3a3]">{{ entry.config?.type ?? "stdio" }}{{ entry.enabled ? "" : "（已停用）" }}</span>
            </span>
            <span class="flex shrink-0 items-center gap-2">
              <button
                type="button"
                class="text-[11px] text-[#2563eb] hover:underline dark:text-[#93c5fd]"
                @click="openMcpForm(entry)"
              >编辑</button>
              <button
                type="button"
                class="text-[11px] text-[#dc2626] hover:underline dark:text-[#fca5a5]"
                @click="deleteAgentMcp(entry)"
              >删除</button>
            </span>
          </div>
          <div v-if="agentMcp.length === 0" class="px-1 text-[11px] text-[#64748b] dark:text-[#a3a3a3]">
            暂无私有 MCP server。
          </div>
        </div>
        <Button variant="outline" size="sm" :class="headerButtonClass" @click="openMcpForm(null)">添加</Button>
      </div>
    </ConfirmDialog>

    <Card v-if="loading" :class="panelClass">
      <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取智能体套件...</CardContent>
    </Card>

    <!-- 卡片网格视图：每个套件一张卡，点击进入配置页；底部行右侧快捷操作 -->
    <div v-else-if="view === 'grid'" class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
      <div
        v-for="item in bundles"
        :key="item.id"
        role="button"
        tabindex="0"
        class="flex min-h-[120px] cursor-pointer flex-col rounded-xl border border-[#e5e7eb] bg-white p-4 text-left transition-all hover:border-[#2563eb]/60 hover:shadow-[0_6px_20px_rgba(37,99,235,0.10)] dark:border-[#333] dark:bg-[#242424] dark:hover:border-[#60a5fa]/50 dark:hover:shadow-none"
        :title="`配置「${item.name}」`"
        @click="openBundleEditor(item.id)"
        @keydown.enter.prevent="openBundleEditor(item.id)"
      >
        <div class="flex items-center justify-between gap-2">
          <span class="truncate text-[14px] font-semibold text-[#111827] dark:text-[#f3f4f6]">{{ item.name }}</span>
          <span
            v-if="item.id === conversationAgentId"
            class="inline-flex shrink-0 items-center gap-1 rounded-full bg-[#f0fdf4] px-2 py-0.5 text-[10.5px] font-medium text-[#15803d] dark:bg-[#052e16]/60 dark:text-[#86efac]"
          >
            <span class="h-1.5 w-1.5 rounded-full bg-[#22c55e]"></span>
            当前对话
          </span>
        </div>
        <p class="mt-1.5 line-clamp-2 flex-1 text-[12.5px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">
          {{ item.description?.trim() || "暂无描述" }}
        </p>
        <div class="mt-2 flex items-center justify-between gap-2">
          <span class="truncate text-[11px] text-[#98a2b3] dark:text-[#9d9589]">更新于 {{ formatUpdatedAt(item.updatedAt) }}</span>
          <span class="flex shrink-0 items-center gap-1.5">
            <button
              type="button"
              class="rounded-md border border-[#d8dee8] bg-white px-2 py-0.5 text-[11.5px] text-[#2563eb] transition-colors hover:bg-[#eff6ff] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#93c5fd] dark:hover:bg-[#2d2d2d]"
              :title="`新开一个对话并启用「${item.name}」`"
              @click.stop="emit('launch-agent', item.id)"
            >启用</button>
            <button
              type="button"
              class="rounded-md border border-[#fecaca] bg-white px-2 py-0.5 text-[11.5px] text-[#dc2626] transition-colors hover:bg-[#fef2f2] dark:border-[#513030] dark:bg-[#242424] dark:text-[#fca5a5] dark:hover:bg-[#3a1f1f]"
              :title="`删除「${item.name}」`"
              @click.stop="openDeleteFromGrid(item)"
            >删除</button>
          </span>
        </div>
      </div>

      <!-- 新建卡片 -->
      <button
        type="button"
        class="flex min-h-[120px] cursor-pointer flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-[#c7d4e8] bg-white/40 p-4 text-[#64748b] transition-colors hover:border-[#2563eb] hover:bg-[#f8fafc] hover:text-[#2563eb] dark:border-[#3f3f3f] dark:bg-transparent dark:text-[#a3a3a3] dark:hover:border-[#60a5fa] dark:hover:bg-[#2a2a2a] dark:hover:text-[#93c5fd]"
        @click="openCreateDialog"
      >
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        <span class="text-[13px] font-medium">新建套件</span>
      </button>
    </div>

    <!-- 配置页视图：单个套件 -->
    <Card v-else-if="draft" :class="panelClass">
      <CardContent class="space-y-4 px-4 pb-4">
        <!-- 顶部：返回 + 套件信息 + 操作 -->
        <div class="flex flex-wrap items-center justify-between gap-3 border-b border-[#e5e7eb] pb-3 dark:border-[#333]">
          <div class="flex min-w-0 items-center gap-2.5">
            <Button variant="outline" size="sm" :class="headerButtonClass" @click="backToGrid">
              ← 返回
            </Button>
            <div class="min-w-0">
              <div class="truncate text-[15px] font-semibold text-[#111827] dark:text-[#f3f4f6]">
                {{ draft.name?.trim() || "未命名套件" }}
              </div>
              <div class="mt-0.5 text-[11.5px] text-[#98a2b3] dark:text-[#9d9589]">
                {{ draft.id === conversationAgentId ? "当前对话正在使用该智能体" : `更新于 ${formatUpdatedAt(draft.updatedAt)}` }}
              </div>
            </div>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <Button
              v-if="isConversationAgent"
              variant="outline"
              size="sm"
              :class="headerButtonClass"
              :disabled="activating"
              title="当前对话正在使用该智能体"
            >
              当前对话使用中
            </Button>
            <Button
              v-else
              size="sm"
              :class="primaryButtonClass"
              :disabled="hasChanges || activating"
              :title="hasChanges ? '请先保存改动' : '新开一个对话并启用该智能体'"
              @click="launchAgent"
            >
              {{ activating ? "处理中..." : "启用" }}
            </Button>
            <Button
              variant="outline"
              size="sm"
              class="h-8 border border-[#fecaca] bg-white px-3 text-[13px] text-[#dc2626] shadow-none hover:bg-[#fef2f2] dark:border-[#513030] dark:bg-[#242424] dark:text-[#fca5a5] dark:hover:bg-[#3a1f1f]"
              @click="showDeletePanel = true"
            >
              删除
            </Button>
          </div>
        </div>

        <!-- 基本信息（直排，无需点开） -->
          <div class="grid grid-cols-[160px_minmax(0,1fr)] gap-2">
            <Input v-model="draft.name" :class="fieldClass" placeholder="套件名称" />
            <Input v-model="draft.description" :class="fieldClass" placeholder="一句话描述（可选）" />
          </div>

          <!-- 系统提示词（直排，无需点开） -->
          <div class="space-y-1.5">
            <div class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">附加系统提示词</div>
            <Textarea
              v-model="draft.prompt"
              class="min-h-[130px] w-full resize-y border-[#d8dee8] bg-white font-mono text-[13px] leading-6 text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#202020] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]"
              spellcheck="false"
              placeholder="## Agent: xxx&#10;角色设定、行为约束、输出格式...（非空时完整替换默认系统提示词）"
            />
          </div>

          <!-- 配置项网格：每个大类一张卡，点击弹窗编辑 -->
          <div class="space-y-2">
            <div class="grid grid-cols-2 gap-2 xl:grid-cols-3">
              <button
                type="button"
                class="config-tile"
                @click="openModal('tools')"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>
                <span class="min-w-0">
                  <span class="config-tile__title">内置工具</span>
                  <span class="config-tile__summary">{{ toolsSummary }}</span>
                </span>
              </button>
              <button
                type="button"
                class="config-tile"
                @click="openModal('skills')"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>
                <span class="min-w-0">
                  <span class="config-tile__title">全局技能</span>
                  <span class="config-tile__summary">{{ skillsSummary }}</span>
                </span>
              </button>
              <button
                type="button"
                class="config-tile"
                @click="openModal('mcp')"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 2v6M15 2v6M6 8h12v4a6 6 0 0 1-12 0z"/><path d="M12 18v4"/></svg>
                <span class="min-w-0">
                  <span class="config-tile__title">全局 MCP</span>
                  <span class="config-tile__summary">{{ mcpSummary }}</span>
                </span>
              </button>
              <button
                type="button"
                class="config-tile"
                @click="openModal('privateSkills')"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>
                <span class="min-w-0">
                  <span class="config-tile__title">私有技能</span>
                  <span class="config-tile__summary">{{ privateSkillsSummary }}</span>
                </span>
              </button>
              <button
                type="button"
                class="config-tile"
                @click="openModal('files')"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                <span class="min-w-0">
                  <span class="config-tile__title">资料文件</span>
                  <span class="config-tile__summary">{{ filesSummary }}</span>
                </span>
              </button>
              <button
                type="button"
                class="config-tile"
                @click="openModal('privateMcp')"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg>
                <span class="min-w-0">
                  <span class="config-tile__title">私有 MCP</span>
                  <span class="config-tile__summary">{{ privateMcpSummary }}</span>
                </span>
              </button>
            </div>
            <div class="text-[11px] text-[#98a2b3] dark:text-[#9d9589]">
              专属内容（私有技能 / 资料文件 / 私有 MCP）只属于该智能体，其他智能体与默认 Nova 不可见。
              <button type="button" class="ml-1 text-[#2563eb] hover:underline dark:text-[#93c5fd]" @click="revealAgentDir">打开目录</button>
            </div>
          </div>

          <!-- 底部：保存操作 -->
          <div class="flex flex-wrap items-center justify-end gap-2 border-t border-[#e5e7eb] pt-3 dark:border-[#333]">
            <span v-if="hasChanges" class="mr-auto text-[12px] text-[#2563eb] dark:text-[#93c5fd]">有未保存改动</span>
            <Button variant="outline" size="sm" :class="headerButtonClass" :disabled="!hasChanges" @click="discardChanges">
              放弃改动
            </Button>
            <Button size="sm" :class="primaryButtonClass" :disabled="!hasChanges || saving" @click="saveBundle">
              {{ saving ? "保存中..." : "保存" }}
            </Button>
          </div>
      </CardContent>
    </Card>
  </div>
</template>

<style scoped>
/* 配置项大类卡片：图标 + 标题 + 当前值摘要 */
.config-tile {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  min-height: 64px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid #e5e7eb;
  background: #fff;
  text-align: left;
  cursor: pointer;
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}
.config-tile:hover {
  border-color: rgba(37, 99, 235, 0.6);
  box-shadow: 0 4px 14px rgba(37, 99, 235, 0.1);
}
.config-tile > svg {
  margin-top: 2px;
  flex-shrink: 0;
  color: #64748b;
}
.config-tile__title {
  display: block;
  font-size: 13px;
  font-weight: 600;
  color: #111827;
}
.config-tile__summary {
  display: block;
  margin-top: 2px;
  font-size: 11px;
  color: #98a2b3;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
:root.dark .config-tile,
html.dark .config-tile {
  background: #242424;
  border-color: #333;
}
:root.dark .config-tile:hover,
html.dark .config-tile:hover {
  border-color: rgba(96, 165, 250, 0.5);
  box-shadow: none;
}
:root.dark .config-tile > svg,
html.dark .config-tile > svg {
  color: #a3a3a3;
}
:root.dark .config-tile__title,
html.dark .config-tile__title {
  color: #f3f4f6;
}
:root.dark .config-tile__summary,
html.dark .config-tile__summary {
  color: #9d9589;
}
</style>
