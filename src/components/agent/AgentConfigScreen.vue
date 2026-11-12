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

type AgentSource = { kind: "manual" | "market" | "import"; name?: string | null; version?: string | null };

type AgentBundle = {
  id: string;
  name: string;
  description: string;
  prompt: string;
  enabledTools: string[] | null;
  enabledSkills: string[] | null;
  enabledMcpServers: string[] | null;
  enabled: boolean;
  source?: AgentSource | null;
  createdAt: number;
  updatedAt: number;
};

type SkillItem = { name: string; description: string; path: string };
type McpServerStatus = { name: string; status: string; type: string; enabled: boolean; sourceAgent?: string | null };

/** 智能体资料文件条目（agents/<id>/files/ 顶层）。 */
type AgentFileEntry = { name: string; sizeBytes: number; isDir: boolean };

interface StarterTemplate {
  name: string;
  badge: string;
  icon: string;
  description: string;
  prompt: string;
}

const STARTER_TEMPLATES: StarterTemplate[] = [
  {
    name: "code-reviewer",
    badge: "代码审查",
    icon: "🔍",
    description: "专注高置信度代码审查与静态审计，挖掘逻辑漏洞、边界异常、空指针风险与并发缺陷，提供精准修复建议。",
    prompt: `你是一名经验丰富的代码审查与审计专家。
你的首要任务是保障代码的正确性、健壮性与安全性。
审查原则：
1. 重点关注：逻辑分支漏洞、空值与越界风险、资源泄露、高并发竞态条件、不符合惯用法的反模式。
2. 杜绝表面样式的琐碎挑刺，只报告置信度高（>=80%）的实质性问题。
3. 发现缺陷时，明确指出具体文件、行号范围、触发路径与修复方案代码。`,
  },
  {
    name: "architecture-explorer",
    badge: "架构调研",
    icon: "🏛️",
    description: "自顶向下剖析复杂工程模块结构、调用链路与数据流转，提炼架构图解并将关键发现发布到协同看板。",
    prompt: `你是一名深谙复杂系统设计的全栈架构调研师。
你的首要任务是梳理系统核心架构、模块边界与关键数据流向。
调研原则：
1. 采用 Top-Down（自顶向下）探索策略，优先查看根目录入口、依赖拓扑与核心接口。
2. 善用协同看板工具（board_post），一旦发现关键架构分层或核心线索，立即发布到 #research 频道。
3. 控制步数预算（10-15步），避免逐个阅读所有细节实现，重点提炼核心设计模式与调用拓扑。`,
  },
  {
    name: "fullstack-developer",
    badge: "全栈开发",
    icon: "⚡",
    description: "多文件协调开发、自动化测试编写与重构，具备全量执行工具链，确保代码能跑通并符合项目规范。",
    prompt: `你是一名敏捷高效的全栈工程师。
开发原则：
1. 严格遵守项目既有架构与编码习惯，优先复用现有工具函数与公共模块。
2. 编写或重构代码后，主动执行单元测试（cargo test / npm test）验证正确性。
3. 遵循零过度包装原则，拒绝脆弱的表面修补，深入根本原因产出高质量实现。`,
  },
];

const props = defineProps<{
  /** 当前对话 id：用于标记哪个对话挂载了哪个智能体。 */
  conversationId: string | null;
}>();

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
  /** 点击「新开对话」：请求新建一个对话并挂载该智能体。 */
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
  | "skills"
  | "mcp"
  | "privateSkills"
  | "files";
const activeModal = ref<EditorModal>("");

const bundles = ref<AgentBundle[]>([]);
const conversationAgentId = ref<string | null>(null);
const selectedId = ref("");

const skills = ref<SkillItem[]>([]);
const mcpServers = ref<McpServerStatus[]>([]);
/** 全局 MCP 目录是否成功加载（区分「加载失败」与「确实为空」，避免误清引用清单）。 */
const mcpCatalogOk = ref(false);

// 编辑态（选中 bundle 的可变副本）
const draft = ref<AgentBundle | null>(null);
const original = ref<string>("");

// 每类扩展能力双模式：false = 全部（跟随全局，存 null）；true = 自定义勾选清单（勾=添加，取消=移除）
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

const activeMountedBundle = computed(
  () => bundles.value.find((b) => b.id === conversationAgentId.value) ?? null,
);

function serializeDraft() {
  if (!draft.value) return "";
  return JSON.stringify({
    name: draft.value.name,
    description: draft.value.description,
    prompt: draft.value.prompt,
    enabledSkills: useCustomSkills.value ? [...skillSelection.value].sort() : null,
    enabledMcpServers: useCustomMcp.value ? [...mcpSelection.value].sort() : null,
    enabled: draft.value.enabled,
  });
}

function applyBundleToEditor(bundle: AgentBundle) {
  draft.value = JSON.parse(JSON.stringify(bundle));
  useCustomSkills.value = bundle.enabledSkills !== null;
  skillSelection.value = new Set(bundle.enabledSkills ?? []);
  useCustomMcp.value = bundle.enabledMcpServers !== null;
  mcpSelection.value = new Set(bundle.enabledMcpServers ?? []);
  original.value = serializeDraft();
}

async function loadCatalog() {
  const [skillList, servers] = await Promise.all([
    invoke<SkillItem[]>("list_skills").catch(() => []),
    invoke<McpServerStatus[]>("get_mcp_server_statuses").catch(() => null),
  ]);
  skills.value = skillList ?? [];
  mcpServers.value = servers ?? [];
  mcpCatalogOk.value = servers !== null;
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

function openBundleEditor(id: string) {
  const bundle = bundles.value.find((b) => b.id === id);
  if (!bundle) return;
  selectedId.value = id;
  applyBundleToEditor(bundle);
  pruneMcpSelection();
  view.value = "editor";
  void loadAgentResources(id);
}

function pruneMcpSelection() {
  if (!mcpCatalogOk.value || !useCustomMcp.value) return;
  const known = new Set(mcpServers.value.map((s) => s.name));
  const pruned = new Set([...mcpSelection.value].filter((n) => known.has(n)));
  if (pruned.size !== mcpSelection.value.size) {
    mcpSelection.value = pruned;
  }
}

// ---------------- 智能体目录资源：私有技能 / 资料文件 ----------------

const privateSkills = ref<SkillItem[]>([]);
const agentFiles = ref<AgentFileEntry[]>([]);
const resourcesLoading = ref(false);

const showMcpForm = ref(false);
const addingGlobalMcp = ref(false);
const mcpForm = ref<{
  name: string;
  kind: "stdio" | "streamable_http";
  command: string;
  args: string;
  url: string;
}>({ name: "", kind: "stdio", command: "", args: "", url: "" });

async function loadAgentResources(bundleId: string) {
  resourcesLoading.value = true;
  try {
    const [skillsRes, filesRes] = await Promise.all([
      invoke<SkillItem[]>("list_agent_private_skills", { bundleId }).catch(() => []),
      invoke<AgentFileEntry[]>("list_agent_files", { bundleId }).catch(() => []),
    ]);
    privateSkills.value = skillsRes ?? [];
    agentFiles.value = filesRes ?? [];
  } finally {
    resourcesLoading.value = false;
  }
}

const pendingDeletePrivateSkill = ref<SkillItem | null>(null);
const deletingPrivateSkill = ref(false);
const deletePrivateSkillOpen = computed({
  get: () => pendingDeletePrivateSkill.value !== null,
  set: (value: boolean) => {
    if (!value && !deletingPrivateSkill.value) pendingDeletePrivateSkill.value = null;
  },
});

function requestDeletePrivateSkill(skill: SkillItem) {
  if (deletingPrivateSkill.value) return;
  pendingDeletePrivateSkill.value = skill;
}

async function confirmDeletePrivateSkill() {
  const skill = pendingDeletePrivateSkill.value;
  if (!skill || !selectedId.value || deletingPrivateSkill.value) return;
  deletingPrivateSkill.value = true;
  try {
    await invoke("delete_skill", { path: skill.path });
    await loadAgentResources(selectedId.value);
  } catch (err) {
    console.error("Failed to delete agent private skill:", err);
    emitToast({ variant: "error", source: "agent-config", message: `删除私有技能失败：${err}` });
  } finally {
    deletingPrivateSkill.value = false;
    pendingDeletePrivateSkill.value = null;
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

const pendingDeleteFileName = ref<string | null>(null);
const deletingAgentFile = ref(false);
const deleteAgentFileOpen = computed({
  get: () => pendingDeleteFileName.value !== null,
  set: (value: boolean) => {
    if (!value && !deletingAgentFile.value) pendingDeleteFileName.value = null;
  },
});

function requestDeleteAgentFile(name: string) {
  if (deletingAgentFile.value) return;
  pendingDeleteFileName.value = name;
}

async function confirmDeleteAgentFile() {
  const name = pendingDeleteFileName.value;
  if (!name || !selectedId.value || deletingAgentFile.value) return;
  deletingAgentFile.value = true;
  try {
    await invoke("delete_agent_file", { bundleId: selectedId.value, name });
    await loadAgentResources(selectedId.value);
  } catch (err) {
    console.error("Failed to delete agent file:", err);
    emitToast({ variant: "error", source: "agent-config", message: `删除资料文件失败：${err}` });
  } finally {
    deletingAgentFile.value = false;
    pendingDeleteFileName.value = null;
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

async function addGlobalMcp() {
  if (addingGlobalMcp.value) return;
  const name = mcpForm.value.name.trim();
  if (!name) {
    emitToast({ variant: "error", source: "agent-config", message: "请输入服务名称。" });
    return;
  }
  let config: Record<string, unknown>;
  if (mcpForm.value.kind === "stdio") {
    const cmd = mcpForm.value.command.trim();
    if (!cmd) {
      emitToast({ variant: "error", source: "agent-config", message: "请输入执行命令。" });
      return;
    }
    const rawArgs = mcpForm.value.args.trim();
    const args = rawArgs ? rawArgs.split(/\s+/) : [];
    config = { command: cmd, args };
  } else {
    const url = mcpForm.value.url.trim();
    if (!url) {
      emitToast({ variant: "error", source: "agent-config", message: "请输入服务 URL。" });
      return;
    }
    config = { url };
  }
  addingGlobalMcp.value = true;
  try {
    await invoke("add_mcp_server", {
      name,
      config,
      sourceAgent: selectedId.value || null,
    });
    showMcpForm.value = false;
    mcpForm.value = { name: "", kind: "stdio", command: "", args: "", url: "" };
    emitToast({ variant: "success", source: "agent-config", message: `已添加全局 MCP 服务「${name}」` });
    mcpServers.value = await invoke<McpServerStatus[]>("get_mcp_server_statuses").catch(
      () => mcpServers.value,
    );
    if (useCustomMcp.value) {
      mcpSelection.value = new Set([...mcpSelection.value, name]);
    }
  } catch (err) {
    console.error("Failed to add global MCP server:", err);
    emitToast({ variant: "error", source: "agent-config", message: `添加失败：${err}` });
  } finally {
    addingGlobalMcp.value = false;
  }
}

async function toggleMcpServerActive(serverName: string, enable: boolean) {
  try {
    await invoke("set_mcp_server_enabled", { name: serverName, enabled: enable });
    mcpServers.value = await invoke<McpServerStatus[]>("get_mcp_server_statuses").catch(
      () => mcpServers.value,
    );
  } catch (err) {
    console.error("Failed to toggle mcp server active:", err);
  }
}

async function toggleBundleEnabled(bundle: AgentBundle) {
  try {
    await invoke("save_agent_bundle", { bundle: { ...bundle, enabled: !bundle.enabled } });
    await loadBundles();
    if (draft.value?.id === bundle.id) {
      draft.value = { ...draft.value, enabled: !bundle.enabled };
      original.value = serializeDraft();
    }
  } catch (err) {
    console.error("Failed to toggle bundle enabled:", err);
  }
}

const sourceLabel = (bundle: AgentBundle) => {
  const kind = bundle.source?.kind ?? "manual";
  if (kind === "market") return "市场";
  if (kind === "import") return "导入";
  return "手动";
};

const formatFileSize = (bytes: number) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
};

const formatUpdatedAt = (ts: number) => {
  if (!ts) return "刚刚";
  const date = new Date(ts * 1000);
  return `${date.getMonth() + 1}/${date.getDate()} ${date.getHours().toString().padStart(2, "0")}:${date.getMinutes().toString().padStart(2, "0")}`;
};

// ---------------- 配置页大类摘要 ----------------

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

function openModal(modal: EditorModal) {
  activeModal.value = modal;
  if ((modal === "privateSkills" || modal === "files") && selectedId.value) {
    void loadAgentResources(selectedId.value);
  }
  if (modal === "mcp") {
    void invoke<McpServerStatus[]>("get_mcp_server_statuses")
      .then((servers) => {
        mcpServers.value = servers ?? [];
        mcpCatalogOk.value = true;
        pruneMcpSelection();
      })
      .catch(() => {});
  }
}

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

type SelectionKey = "skills" | "mcp";

function toggleSelection(key: SelectionKey, name: string) {
  const target = key === "skills" ? skillSelection : mcpSelection;
  const next = new Set(target.value);
  if (next.has(name)) {
    next.delete(name);
  } else {
    next.add(name);
  }
  target.value = next;
}

function enableCustomMode(key: SelectionKey) {
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
  if (key === "skills") {
    skillSelection.value = all ? new Set(skills.value.map((s) => s.name)) : new Set();
    return;
  }
  mcpSelection.value = all ? new Set(mcpServers.value.map((s) => s.name)) : new Set();
}

function openCreateDialog() {
  newBundleName.value = "";
  showCreatePanel.value = true;
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
    if (created?.id) openBundleEditor(created.id);
    emitToast({ variant: "success", source: "agent-config", message: "已创建智能体套件。" });
  } catch (err) {
    console.error("Failed to create agent bundle:", err);
  } finally {
    creating.value = false;
  }
}

async function createBundleFromTemplate(tpl: StarterTemplate) {
  if (creating.value) return;
  creating.value = true;
  try {
    const created = await invoke<AgentBundle>("create_agent_bundle", { name: tpl.name });
    if (created?.id) {
      const populated: AgentBundle = {
        ...created,
        description: tpl.description,
        prompt: tpl.prompt,
        enabledTools: null,
      };
      await invoke("save_agent_bundle", { bundle: populated });
      await loadBundles();
      openBundleEditor(created.id);
      emitToast({ variant: "success", source: "agent-config", message: `已基于模版成功创建「${tpl.name}」` });
    }
  } catch (err) {
    console.error("Failed to create template bundle:", err);
    emitToast({ variant: "error", source: "agent-config", message: `创建模版套件失败：${err}` });
  } finally {
    creating.value = false;
  }
}

async function mountToCurrentConversation(bundleId: string) {
  if (!props.conversationId) return;
  try {
    await invoke("set_conversation_agent", {
      conversationId: props.conversationId,
      bundleId,
    });
    conversationAgentId.value = bundleId;
    window.dispatchEvent(new CustomEvent("agent-bundle-changed"));
    emitToast({ variant: "success", source: "agent-config", message: "已将该智能体挂载到当前对话。" });
  } catch (err) {
    console.error("Failed to mount agent to conversation:", err);
    emitToast({ variant: "error", source: "agent-config", message: `挂载失败：${err}` });
  }
}

async function unmountFromCurrentConversation() {
  if (!props.conversationId) return;
  try {
    await invoke("set_conversation_agent", {
      conversationId: props.conversationId,
      bundleId: null,
    });
    conversationAgentId.value = null;
    window.dispatchEvent(new CustomEvent("agent-bundle-changed"));
    emitToast({ variant: "success", source: "agent-config", message: "已恢复默认 Nova 助手。" });
  } catch (err) {
    console.error("Failed to unmount agent:", err);
    emitToast({ variant: "error", source: "agent-config", message: `恢复失败：${err}` });
  }
}

async function saveBundle() {
  if (!draft.value || saving.value) return;
  saving.value = true;
  try {
    const bundle: AgentBundle = {
      ...draft.value,
      enabledTools: null, // 内置工具由底座全量保障，永不剔除
      enabledSkills: useCustomSkills.value ? [...skillSelection.value] : null,
      enabledMcpServers: useCustomMcp.value ? [...mcpSelection.value] : null,
    };
    const saved = await invoke<AgentBundle>("save_agent_bundle", { bundle });
    await loadBundles();
    if (saved) applyBundleToEditor(saved);
    emitToast({ variant: "success", source: "agent-config", message: "智能体套件已保存。" });
  } catch (err) {
    console.error("Failed to save agent bundle:", err);
  } finally {
    saving.value = false;
  }
}

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
  window.setTimeout(() => {
    activating.value = false;
  }, 600);
}

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

function getBundleIcon(name: string) {
  const n = name.toLowerCase();
  if (n.includes("review") || n.includes("audit") || n.includes("审")) return "🔍";
  if (n.includes("arch") || n.includes("explore") || n.includes("构")) return "🏛️";
  if (n.includes("full") || n.includes("dev") || n.includes("全栈")) return "⚡";
  if (n.includes("test") || n.includes("测")) return "🧪";
  return "🤖";
}

onMounted(async () => {
  await Promise.all([loadBundles(), loadCatalog(), loadConversationAgent()]);
});
</script>

<template>
  <div :class="pageClass">
    <!-- 头部栏 -->
    <header v-if="view === 'grid'" class="flex flex-wrap items-start justify-between gap-3 border-b border-[#e5e7eb] pb-3 dark:border-[#333]">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">智能体工坊</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
          为特定工程场景定制专家 AI，装备专有人设、外部 MCP 扩展与业务资料。内置基础工具全量保障。
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button size="sm" :class="primaryButtonClass" :disabled="creating" @click="openCreateDialog">
          + 新建空白套件
        </Button>
        <Button variant="outline" size="sm" :class="headerButtonClass" @click="emit('change-main-view', 'chat')">
          返回聊天
        </Button>
      </div>
    </header>

    <!-- 当前会话挂载状态提示条 -->
    <div
      v-if="view === 'grid' && hasConversation && activeMountedBundle"
      class="flex items-center justify-between gap-3 rounded-lg border border-[#bbf7d0] bg-[#f0fdf4] px-3.5 py-2 text-[12.5px] text-[#15803d] dark:border-[#14532d] dark:bg-[#052e16]/60 dark:text-[#86efac]"
    >
      <div class="flex items-center gap-2">
        <span class="h-2 w-2 rounded-full bg-[#22c55e]"></span>
        <span>当前对话正在运行专属智能体：<strong>「{{ activeMountedBundle.name }}」</strong></span>
      </div>
      <button
        type="button"
        class="text-[12px] underline hover:text-[#166534] dark:hover:text-[#bbf7d0]"
        @click="unmountFromCurrentConversation"
      >
        恢复默认 Nova 助手
      </button>
    </div>

    <!-- 新建套件弹窗 -->
    <ConfirmDialog
      v-model="showCreatePanel"
      title="新建智能体套件"
      description="新建套件内置基础工具全量可用，之后可自由添加专有提示词、挂载资料与 MCP 服务。"
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
      />
    </ConfirmDialog>

    <!-- 删除确认弹窗 -->
    <ConfirmDialog
      v-model="showDeletePanel"
      title="删除智能体套件"
      :description="`将删除「${selectedBundle?.name ?? ''}」整套配置，所有挂载它的对话自动回到默认 Nova。不可恢复。`.replace(/\\n/g, ' ')"
      confirm-text="删除"
      cancel-text="取消"
      destructive
      @confirm="deleteBundle"
    />

    <!-- 私有技能删除确认弹窗 -->
    <ConfirmDialog
      v-model="deletePrivateSkillOpen"
      title="删除私有技能"
      :description="`确定删除私有技能「${pendingDeletePrivateSkill?.name ?? ''}」？此操作不可恢复。`.replace(/\\n/g, ' ')"
      confirm-text="删除"
      cancel-text="取消"
      :busy="deletingPrivateSkill"
      destructive
      @confirm="confirmDeletePrivateSkill"
    />

    <!-- 资料文件删除确认弹窗 -->
    <ConfirmDialog
      v-model="deleteAgentFileOpen"
      title="删除资料"
      :description="`确定删除资料「${pendingDeleteFileName ?? ''}」？此操作不可恢复。`.replace(/\\n/g, ' ')"
      confirm-text="删除"
      cancel-text="取消"
      :busy="deletingAgentFile"
      destructive
      @confirm="confirmDeleteAgentFile"
    />

    <!-- 添加全局 MCP 弹窗 -->
    <ConfirmDialog
      v-model="showMcpForm"
      width-class="max-w-[560px]"
      title="添加 MCP 服务"
      description="保存后写入全局 MCP 注册表，自动引用给当前智能体并立即连接。"
      :confirm-text="addingGlobalMcp ? '连接中...' : '保存'"
      cancel-text="取消"
      :busy="addingGlobalMcp"
      @confirm="addGlobalMcp"
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
        <div class="text-[11px] text-[#98a2b3] dark:text-[#a3a3a3]">
          需要高级字段（env/headers）时可在「设置 → MCP」页编辑。
        </div>
      </div>
    </ConfirmDialog>

    <!-- 全局技能弹窗 -->
    <ConfirmDialog
      :model-value="activeModal === 'skills'"
      width-class="max-w-[640px]"
      title="配置全局技能装备"
      description="已启用的全局技能对该智能体生效。默认全量可用，亦可自定义精简。"
      confirm-text="完成"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      @confirm="activeModal = ''"
    >
      <div class="space-y-3">
        <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
          <button
            type="button"
            class="flex-1 px-3 py-1.5 transition-colors"
            :class="!useCustomSkills ? modeTabActiveClass : modeTabIdleClass"
            @click="useCustomSkills = false"
          >全部跟随全局（{{ skills.length }}）</button>
          <button
            type="button"
            class="flex-1 border-l border-[#d8dee8] px-3 py-1.5 transition-colors dark:border-[#3a3a3a]"
            :class="useCustomSkills ? modeTabActiveClass : modeTabIdleClass"
            @click="enableCustomMode('skills')"
          >自定义（{{ skillSelection.size }}）</button>
        </div>

        <template v-if="useCustomSkills">
          <div class="flex items-center justify-between">
            <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">勾选 = 装备该技能</span>
            <div class="flex gap-1">
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('skills', true)">全选</Button>
              <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('skills', false)">清空</Button>
            </div>
          </div>
          <div class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
            <label
              v-for="skill in skills"
              :key="skill.name"
              class="flex cursor-pointer items-start gap-2.5 rounded-md px-2 py-1.5 transition-colors hover:bg-[#f4f7fb] dark:hover:bg-[#2a2a2a]"
            >
              <input
                type="checkbox"
                class="mt-1 h-3.5 w-3.5 rounded border-[#cbd5e1] text-[#2563eb] dark:border-[#525252]"
                :checked="skillSelection.has(skill.name)"
                @change="toggleSelection('skills', skill.name)"
              />
              <span class="min-w-0 flex-1">
                <span class="text-[13px] font-medium text-[#111827] dark:text-[#ededed]">{{ skill.name }}</span>
                <span class="line-clamp-2 block text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">{{ skill.description }}</span>
              </span>
            </label>
          </div>
        </template>
      </div>
    </ConfirmDialog>

    <!-- 全局 MCP 弹窗 -->
    <ConfirmDialog
      :model-value="activeModal === 'mcp'"
      width-class="max-w-[640px]"
      title="配置全局 MCP 服务装备"
      description="勾选本智能体需要连接的外部 MCP 服务。可在下方直接新增 MCP 服务。"
      confirm-text="完成"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      @confirm="activeModal = ''"
    >
      <div class="space-y-3">
        <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
          <button
            type="button"
            class="flex-1 px-3 py-1.5 transition-colors"
            :class="!useCustomMcp ? modeTabActiveClass : modeTabIdleClass"
            @click="useCustomMcp = false"
          >全部已启用服务（{{ mcpServers.length }}）</button>
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
              class="flex cursor-pointer items-center justify-between gap-2.5 rounded-md px-2 py-1.5 transition-colors hover:bg-[#f4f7fb] dark:hover:bg-[#2a2a2a]"
            >
              <span class="flex items-center gap-2">
                <input
                  type="checkbox"
                  class="h-3.5 w-3.5 rounded border-[#cbd5e1] text-[#2563eb] dark:border-[#525252]"
                  :checked="mcpSelection.has(server.name)"
                  @change="toggleSelection('mcp', server.name)"
                />
                <span class="text-[13px] font-medium text-[#111827] dark:text-[#ededed]">{{ server.name }}</span>
                <span class="text-[11px] text-[#98a2b3] dark:text-[#a3a3a3]">({{ server.type }})</span>
              </span>
              <button
                type="button"
                class="text-[11px] transition-colors"
                :class="server.enabled ? 'text-[#15803d] hover:underline' : 'text-[#64748b] hover:underline'"
                @click.stop="toggleMcpServerActive(server.name, !server.enabled)"
              >
                {{ server.enabled ? "运行中" : "已停用" }}
              </button>
            </label>
          </div>
        </template>
        <div class="flex justify-end">
          <Button variant="outline" size="sm" :class="headerButtonClass" @click="showMcpForm = true">
            + 添加新 MCP 服务
          </Button>
        </div>
      </div>
    </ConfirmDialog>

    <!-- 私有技能弹窗 -->
    <ConfirmDialog
      :model-value="activeModal === 'privateSkills'"
      width-class="max-w-[640px]"
      title="私有专属技能"
      description="存放于 agents/<id>/skills/，只对该智能体可见。不受全局技能白名单过滤。"
      confirm-text="完成"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      @confirm="activeModal = ''"
    >
      <div class="space-y-3">
        <div v-if="privateSkills.length === 0" class="py-4 text-center text-sm text-[#98a2b3] dark:text-[#a3a3a3]">
          暂无私有技能。点击下方按钮打开目录放入 SKILL.md 技能包。
        </div>
        <div v-else class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
          <div
            v-for="s in privateSkills"
            :key="s.name"
            class="flex items-center justify-between rounded px-2 py-1.5 hover:bg-[#f4f7fb] dark:hover:bg-[#2a2a2a]"
          >
            <div>
              <div class="text-[13px] font-medium text-[#111827] dark:text-[#ededed]">{{ s.name }}</div>
              <div class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">{{ s.description }}</div>
            </div>
            <button
              type="button"
              class="text-[11.5px] text-[#dc2626] hover:underline"
              @click="requestDeletePrivateSkill(s)"
            >
              删除
            </button>
          </div>
        </div>
        <div class="flex justify-end">
          <Button variant="outline" size="sm" :class="headerButtonClass" @click="revealAgentDir">
            打开智能体目录
          </Button>
        </div>
      </div>
    </ConfirmDialog>

    <!-- 资料文件弹窗 -->
    <ConfirmDialog
      :model-value="activeModal === 'files'"
      width-class="max-w-[640px]"
      title="专有参考资料"
      description="文件存放于 agents/<id>/files/。模型可通过内置 Read/Grep/Glob 直接读取这些资料。"
      confirm-text="完成"
      @update:model-value="(v) => { if (!v) activeModal = '' }"
      @confirm="activeModal = ''"
    >
      <div class="space-y-3">
        <div v-if="agentFiles.length === 0" class="py-4 text-center text-sm text-[#98a2b3] dark:text-[#a3a3a3]">
          暂无参考资料。可点击「导入文件」将规范、架构图或 API 文档导入给该智能体。
        </div>
        <div v-else class="space-y-1 rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333]">
          <div
            v-for="f in agentFiles"
            :key="f.name"
            class="flex items-center justify-between rounded px-2 py-1.5 hover:bg-[#f4f7fb] dark:hover:bg-[#2a2a2a]"
          >
            <div class="flex items-center gap-2">
              <span class="text-[14px]">📄</span>
              <span class="text-[13px] font-medium text-[#111827] dark:text-[#ededed]">{{ f.name }}</span>
              <span class="text-[11px] text-[#98a2b3] dark:text-[#a3a3a3]">({{ formatFileSize(f.sizeBytes) }})</span>
            </div>
            <button
              type="button"
              class="text-[11.5px] text-[#dc2626] hover:underline"
              @click="requestDeleteAgentFile(f.name)"
            >
              删除
            </button>
          </div>
        </div>
        <div class="flex justify-between">
          <Button variant="outline" size="sm" :class="headerButtonClass" @click="importAgentFile">导入资料文件</Button>
          <Button variant="outline" size="sm" :class="headerButtonClass" @click="revealAgentDir">打开目录</Button>
        </div>
      </div>
    </ConfirmDialog>

    <!-- 加载中状态 -->
    <Card v-if="loading" :class="panelClass">
      <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取智能体工坊数据...</CardContent>
    </Card>

    <!-- 卡片网格视图 -->
    <div v-else-if="view === 'grid'" class="space-y-6">
      <!-- 空状态引导区：当尚未创建任何智能体时，展示推荐模版 -->
      <div v-if="bundles.length === 0" class="space-y-4">
        <div class="rounded-xl border border-[#d8dee8] bg-[#f8fafc] p-5 text-center dark:border-[#333] dark:bg-[#242424]">
          <h3 class="text-[15px] font-semibold text-[#111827] dark:text-[#ededed]">尚未创建任何自定义智能体</h3>
          <p class="mt-1 text-[13px] text-[#64748b] dark:text-[#a3a3a3]">
            您可以直接选用下方针对特定场景精心调优的开箱即用专家模版，或从空白自由创建：
          </p>
        </div>

        <div class="grid grid-cols-1 gap-3.5 md:grid-cols-3">
          <div
            v-for="tpl in STARTER_TEMPLATES"
            :key="tpl.name"
            class="flex flex-col justify-between rounded-xl border border-[#e5e7eb] bg-white p-4 transition-all hover:border-[#2563eb]/60 hover:shadow-md dark:border-[#333] dark:bg-[#202020]"
          >
            <div>
              <div class="flex items-center justify-between">
                <span class="text-2xl">{{ tpl.icon }}</span>
                <span class="rounded-full bg-[#eff6ff] px-2 py-0.5 text-[11px] font-medium text-[#2563eb] dark:bg-[#1e3a8a]/40 dark:text-[#93c5fd]">
                  {{ tpl.badge }}
                </span>
              </div>
              <h4 class="mt-2 text-[14px] font-semibold text-[#111827] dark:text-[#ededed]">{{ tpl.name }}</h4>
              <p class="mt-1.5 text-[12px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">{{ tpl.description }}</p>
            </div>
            <div class="mt-4 pt-3 border-t border-[#f1f5f9] dark:border-[#2d2d2d]">
              <Button
                size="sm"
                :class="primaryButtonClass"
                class="w-full text-center"
                :disabled="creating"
                @click="createBundleFromTemplate(tpl)"
              >
                基于此模版创建
              </Button>
            </div>
          </div>
        </div>
      </div>

      <!-- 用户已有智能体套件列表 -->
      <div v-else class="grid grid-cols-1 gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
        <div
          v-for="item in bundles"
          :key="item.id"
          role="button"
          tabindex="0"
          class="flex min-h-[140px] cursor-pointer flex-col justify-between rounded-xl border border-[#e5e7eb] bg-white p-4 text-left transition-all hover:border-[#2563eb]/60 hover:shadow-sm dark:border-[#333] dark:bg-[#242424] dark:hover:border-[#60a5fa]/50"
          :class="item.enabled ? '' : 'opacity-65'"
          :title="`配置「${item.name}」`"
          @click="openBundleEditor(item.id)"
          @keydown.enter.prevent="openBundleEditor(item.id)"
        >
          <div>
            <div class="flex items-start justify-between gap-2">
              <div class="flex items-center gap-2">
                <span class="text-xl">{{ getBundleIcon(item.name) }}</span>
                <span class="truncate text-[14.5px] font-semibold text-[#111827] dark:text-[#f3f4f6]">{{ item.name }}</span>
              </div>
              <div class="flex shrink-0 items-center gap-1.5">
                <span class="inline-flex items-center rounded-full bg-[#f3f4f6] px-2 py-0.5 text-[10.5px] font-medium text-[#6b7280] dark:bg-[#2e2e2e] dark:text-[#a3a3a3]">
                  {{ sourceLabel(item) }}
                </span>
                <span
                  v-if="item.id === conversationAgentId"
                  class="inline-flex items-center gap-1 rounded-full bg-[#f0fdf4] px-2 py-0.5 text-[10.5px] font-medium text-[#15803d] dark:bg-[#052e16]/60 dark:text-[#86efac]"
                >
                  <span class="h-1.5 w-1.5 rounded-full bg-[#22c55e]"></span>
                  当前会话挂载
                </span>
              </div>
            </div>

            <p class="mt-2 line-clamp-2 text-[12.5px] leading-5 text-[#64748b] dark:text-[#a3a3a3]">
              {{ item.description?.trim() || "专精定制专家角色，全量基础工具已就绪。" }}
            </p>

            <div class="mt-2.5 flex flex-wrap gap-1.5 text-[11px] text-[#64748b] dark:text-[#a3a3a3]">
              <span class="rounded bg-[#f1f5f9] px-1.5 py-0.5 dark:bg-[#2d2d2d]">
                🔌 MCP: {{ item.enabledMcpServers ? item.enabledMcpServers.length : '全部' }}
              </span>
              <span class="rounded bg-[#f1f5f9] px-1.5 py-0.5 dark:bg-[#2d2d2d]">
                ⚡ 技能: {{ item.enabledSkills ? item.enabledSkills.length : '全部' }}
              </span>
            </div>
          </div>

          <div class="mt-3 flex items-center justify-between gap-2 border-t border-[#f1f5f9] pt-2.5 dark:border-[#2d2d2d]">
            <span class="truncate text-[11px] text-[#98a2b3] dark:text-[#a3a3a3]">
              更新于 {{ formatUpdatedAt(item.updatedAt) }}
            </span>
            <span class="flex shrink-0 items-center gap-1.5">
              <button
                type="button"
                class="rounded-md border px-2 py-0.5 text-[11.5px] transition-colors"
                :class="item.enabled
                  ? 'border-[#bbf7d0] bg-[#f0fdf4] text-[#15803d] hover:bg-[#dcfce7] dark:border-[#14532d] dark:bg-[#052e16]/60 dark:text-[#86efac]' 
                  : 'border-[#d8dee8] bg-white text-[#64748b] hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#a3a3a3]'"
                :title="item.enabled ? '点击休眠：休眠后不可被会话挂载' : '点击上架启用'"
                @click.stop="toggleBundleEnabled(item)"
              >
                {{ item.enabled ? '已上架' : '已休眠' }}
              </button>

              <button
                v-if="hasConversation && item.id !== conversationAgentId && item.enabled"
                type="button"
                class="rounded-md border border-[#93c5fd] bg-[#eff6ff] px-2 py-0.5 text-[11.5px] text-[#1d4ed8] transition-colors hover:bg-[#dbeafe] dark:border-[#1e40af] dark:bg-[#1e3a8a]/40 dark:text-[#93c5fd]"
                title="直接在当前对话中挂载此智能体，无须新开窗口"
                @click.stop="mountToCurrentConversation(item.id)"
              >
                挂载当前
              </button>

              <button
                type="button"
                class="rounded-md border border-[#d8dee8] bg-white px-2 py-0.5 text-[11.5px] text-[#2563eb] transition-colors hover:bg-[#eff6ff] disabled:opacity-50 dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#93c5fd]"
                :title="item.enabled ? `新开对话使用「${item.name}」` : '智能体已休眠'"
                :disabled="!item.enabled"
                @click.stop="emit('launch-agent', item.id)"
              >
                新开对话
              </button>

              <button
                type="button"
                class="rounded-md border border-[#fecaca] bg-white px-2 py-0.5 text-[11.5px] text-[#dc2626] transition-colors hover:bg-[#fef2f2] dark:border-[#513030] dark:bg-[#242424] dark:text-[#fca5a5]"
                @click.stop="openDeleteFromGrid(item)"
              >
                删除
              </button>
            </span>
          </div>
        </div>

        <!-- 新建卡片 -->
        <button
          type="button"
          class="flex min-h-[140px] cursor-pointer flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-[#c7d4e8] bg-white/40 p-4 text-[#64748b] transition-colors hover:border-[#2563eb] hover:bg-[#f8fafc] hover:text-[#2563eb] dark:border-[#3f3f3f] dark:bg-transparent dark:text-[#a3a3a3] dark:hover:border-[#60a5fa] dark:hover:bg-[#2a2a2a] dark:hover:text-[#93c5fd]"
          @click="openCreateDialog"
        >
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
          <span class="text-[13px] font-medium">新建空白套件</span>
        </button>
      </div>

      <!-- 底部模版快捷入口（当用户已有套件时，依然可在此基于模版快速派生） -->
      <div v-if="bundles.length > 0" class="mt-8 border-t border-[#e5e7eb] pt-5 dark:border-[#333]">
        <h4 class="text-[13px] font-medium text-[#475569] dark:text-[#a3a3a3]">💡 推荐专家模版库</h4>
        <div class="mt-2.5 grid grid-cols-1 gap-2.5 sm:grid-cols-3">
          <div
            v-for="tpl in STARTER_TEMPLATES"
            :key="tpl.name"
            class="flex items-center justify-between rounded-lg border border-[#e5e7eb] bg-[#fafafa] px-3 py-2 text-left dark:border-[#333] dark:bg-[#202020]"
          >
            <div class="min-w-0 pr-2">
              <span class="text-[13px] font-medium text-[#111827] dark:text-[#ededed]">{{ tpl.icon }} {{ tpl.name }}</span>
              <span class="block truncate text-[11px] text-[#64748b] dark:text-[#a3a3a3]">{{ tpl.description }}</span>
            </div>
            <button
              type="button"
              class="shrink-0 text-[11.5px] text-[#2563eb] hover:underline dark:text-[#93c5fd]"
              :disabled="creating"
              @click="createBundleFromTemplate(tpl)"
            >
              选用
            </button>
          </div>
        </div>
      </div>
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
              v-if="hasConversation && !isConversationAgent && draft.enabled"
              size="sm"
              variant="outline"
              class="h-8 border-[#93c5fd] bg-[#eff6ff] text-[13px] text-[#1d4ed8] shadow-none hover:bg-[#dbeafe] dark:border-[#1e40af] dark:bg-[#1e3a8a]/40 dark:text-[#93c5fd]"
              @click="mountToCurrentConversation(draft.id)"
            >
              挂载到当前对话
            </Button>

            <Button
              v-if="isConversationAgent"
              variant="outline"
              size="sm"
              :class="headerButtonClass"
              title="当前对话正在使用该智能体"
              @click="unmountFromCurrentConversation"
            >
              已在当前对话 (点击解除)
            </Button>

            <Button
              size="sm"
              :class="primaryButtonClass"
              :disabled="hasChanges || activating || !draft.enabled"
              :title="!draft.enabled ? '智能体已休眠' : hasChanges ? '请先保存改动' : '新开一个对话并启用该智能体'"
              @click="launchAgent"
            >
              {{ activating ? "处理中..." : "新开对话使用" }}
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

        <!-- 基本信息 -->
        <div class="grid grid-cols-[160px_minmax(0,1fr)] gap-2">
          <Input v-model="draft.name" :class="fieldClass" placeholder="套件名称" />
          <Input v-model="draft.description" :class="fieldClass" placeholder="一句话描述（可选）" />
        </div>

        <!-- 系统提示词（Overlay 模式） -->
        <div class="space-y-1.5">
          <div class="flex items-center justify-between">
            <span class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">专有提示词（Prompt Overlay 模式）</span>
            <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">底层工程协议恒常自洽，此处只需注入专属目标</span>
          </div>
          <Textarea
            v-model="draft.prompt"
            class="min-h-[140px] w-full resize-y border-[#d8dee8] bg-white font-mono text-[13px] leading-6 text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#202020] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]"
            spellcheck="false"
            placeholder="在这里输入专属于该角色的行为准则、审查关注点、输出格式要求...\n（以追加叠加模式生效，底层文件读写/终端/协同规范永远完整保留）"
          />
        </div>

        <!-- 扩展能力与专属资料卡片 -->
        <div class="space-y-2">
          <div class="flex items-center gap-1.5 rounded-lg border border-[#e2e8f0] bg-[#f8fafc] px-3 py-2 text-[12px] text-[#475569] dark:border-[#333] dark:bg-[#202020] dark:text-[#a3a3a3]">
            <span>🛡️</span>
            <span><strong>内置基础工具全量保障</strong>：Read、Write、Edit、apply_patch、Grep、Glob、交互终端（exec_command / write_stdin）与协同看板由底座保障永远在线，无需手动配置。</span>
          </div>

          <div class="grid grid-cols-2 gap-2 xl:grid-cols-4">
            <button
              type="button"
              class="config-tile"
              @click="openModal('mcp')"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 2v6M15 2v6M6 8h12v4a6 6 0 0 1-12 0z"/><path d="M12 18v4"/></svg>
              <span class="min-w-0">
                <span class="config-tile__title">全局 MCP 服务</span>
                <span class="config-tile__summary">{{ mcpSummary }}</span>
              </span>
            </button>
            <button
              type="button"
              class="config-tile"
              @click="openModal('skills')"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>
              <span class="min-w-0">
                <span class="config-tile__title">全局技能库</span>
                <span class="config-tile__summary">{{ skillsSummary }}</span>
              </span>
            </button>
            <button
              type="button"
              class="config-tile"
              @click="openModal('privateSkills')"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>
              <span class="min-w-0">
                <span class="config-tile__title">私有专属技能</span>
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
                <span class="config-tile__title">专属参考资料</span>
                <span class="config-tile__summary">{{ filesSummary }}</span>
              </span>
            </button>
          </div>
          <div class="text-[11px] text-[#98a2b3] dark:text-[#a3a3a3]">
            专属内容（私有技能 / 资料文件）只属于该智能体；MCP 服务为全局注册，上方维护接入清单。
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
.config-tile {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  padding: 0.625rem 0.75rem;
  border-radius: 0.5rem;
  border: 1px solid #e5e7eb;
  background-color: #fff;
  text-align: left;
  transition: all 0.15s ease;
  cursor: pointer;
}
.config-tile:hover {
  border-color: #2563eb;
  background-color: #f8fafc;
}
:deep(.dark) .config-tile,
.dark .config-tile {
  border-color: #333;
  background-color: #202020;
}
:deep(.dark) .config-tile:hover,
.dark .config-tile:hover {
  border-color: #60a5fa;
  background-color: #2a2a2a;
}
.config-tile__title {
  display: block;
  font-size: 13px;
  font-weight: 500;
  color: #111827;
}
:deep(.dark) .config-tile__title,
.dark .config-tile__title {
  color: #ededed;
}
.config-tile__summary {
  display: block;
  font-size: 11px;
  color: #64748b;
}
:deep(.dark) .config-tile__summary,
.dark .config-tile__summary {
  color: #a3a3a3;
}
</style>
