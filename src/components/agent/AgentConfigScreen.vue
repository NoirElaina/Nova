<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
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

async function loadBundles(selectId?: string) {
  loading.value = true;
  try {
    const items = await invoke<AgentBundle[]>("list_agent_bundles");
    bundles.value = items ?? [];

    if (bundles.value.length === 0) {
      selectedId.value = "";
      draft.value = null;
      original.value = "";
      return;
    }

    const target =
      selectId && bundles.value.some((b) => b.id === selectId)
        ? selectId
        : bundles.value.some((b) => b.id === selectedId.value)
          ? selectedId.value
          : bundles.value[0].id;
    selectedId.value = target;
    const bundle = bundles.value.find((b) => b.id === target);
    if (bundle) applyBundleToEditor(bundle);
  } catch (err) {
    console.error("Failed to load agent bundles:", err);
  } finally {
    loading.value = false;
  }
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

function handleSelectBundle(id: string) {
  if (!id || id === selectedId.value) return;
  if (hasChanges.value) {
    emitToast({
      variant: "error",
      source: "agent-config",
      message: "当前有未保存改动，请先保存或放弃后再切换。",
    });
    return;
  }
  selectedId.value = id;
  const bundle = bundles.value.find((b) => b.id === id);
  if (bundle) applyBundleToEditor(bundle);
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
    await loadBundles(created?.id);
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
    await loadBundles(saved?.id ?? bundle.id);
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

/** 点击 Nova 默认项：当前对话挂载了智能体时卸载，复位为默认 Nova。 */
async function resetConversationToNova() {
  if (conversationAgentId.value === null || activating.value) return;
  if (!hasConversation.value) return;
  activating.value = true;
  try {
    await invoke("set_conversation_agent", {
      conversationId: props.conversationId,
      bundleId: null,
    });
    conversationAgentId.value = null;
    emitToast({
      variant: "success",
      source: "agent-config",
      message: "当前对话已切回默认 Nova。",
    });
    window.dispatchEvent(new CustomEvent("agent-bundle-changed"));
  } catch (err) {
    console.error("Failed to reset conversation agent:", err);
  } finally {
    activating.value = false;
  }
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
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">智能体套件</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
          每个套件 = 附加提示词 + 工具/技能/MCP 装备清单。点击「启用」新开一个对话并使用该智能体。
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

    <Card v-if="loading" :class="panelClass">
      <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取智能体套件...</CardContent>
    </Card>

    <div v-else class="grid min-h-[420px] flex-1 grid-cols-[200px_minmax(0,1fr)] gap-3">
      <!-- 左：套件列表（原生 button，明确的选中/悬浮反馈） -->
      <Card :class="panelClass">
        <CardHeader class="space-y-1 px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">套件</CardTitle>
          <CardDescription>共 {{ bundles.length }} 个</CardDescription>
        </CardHeader>
        <CardContent class="px-2.5">
          <div class="max-h-[calc(100vh-300px)] space-y-1 overflow-y-auto pr-1 custom-scrollbar">
            <!-- Nova 默认项：当前对话未挂载智能体时绿点亮起；点击可把对话复位为默认 Nova -->
            <button
              type="button"
              class="block w-full cursor-pointer rounded-lg border px-2.5 py-2 text-left transition-colors"
              :class="conversationAgentId === null
                ? 'border-[#86efac] bg-[#f0fdf4] hover:bg-[#f0fdf4] dark:border-[#166534] dark:bg-[#052e16]/40 dark:hover:bg-[#052e16]/40'
                : 'border-transparent hover:border-[#c7d4e8] hover:bg-[#f4f7fb] dark:border-transparent dark:hover:border-[#3f3f3f] dark:hover:bg-[#2a2a2a]'"
              :title="conversationAgentId ? '点击将当前对话复位为默认 Nova' : '当前对话正在使用默认 Nova'"
              @click="resetConversationToNova"
            >
              <div class="flex items-center gap-1.5">
                <span
                  class="inline-block h-1.5 w-1.5 shrink-0 rounded-full"
                  :class="conversationAgentId === null ? 'bg-[#22c55e]' : 'bg-transparent'"
                ></span>
                <span
                  class="truncate text-[13px] font-medium"
                  :class="conversationAgentId === null
                    ? 'text-[#15803d] dark:text-[#86efac]'
                    : 'text-[#111827] dark:text-[#e2dbcf]'"
                >Nova（默认）</span>
              </div>
              <div class="mt-0.5 truncate pl-3 text-[11px] text-[#98a2b3] dark:text-[#9d9589]">
                {{ conversationAgentId === null ? '当前对话使用中' : '全部能力，无附加配置' }}
              </div>
            </button>

            <button
              v-for="item in bundles"
              :key="item.id"
              type="button"
              class="block w-full cursor-pointer rounded-lg border px-2.5 py-2 text-left transition-colors"
              :class="item.id === selectedId
                ? 'border-[#2563eb] bg-[#eff6ff] dark:border-[#1d4ed8] dark:bg-[#1e293b]'
                : 'border-transparent hover:border-[#c7d4e8] hover:bg-[#f4f7fb] dark:border-transparent dark:hover:border-[#3f3f3f] dark:hover:bg-[#2a2a2a]'"
              @click="handleSelectBundle(item.id)"
            >
              <div class="flex items-center gap-1.5">
                <span
                  class="inline-block h-1.5 w-1.5 shrink-0 rounded-full"
                  :class="item.id === conversationAgentId ? 'bg-[#22c55e]' : 'bg-transparent'"
                ></span>
                <span
                  class="truncate text-[13px] font-medium"
                  :class="item.id === selectedId
                    ? 'text-[#1d4ed8] dark:text-[#93c5fd]'
                    : 'text-[#111827] dark:text-[#e2dbcf]'"
                >{{ item.name }}</span>
              </div>
              <div class="mt-0.5 truncate pl-3 text-[11px] text-[#98a2b3] dark:text-[#9d9589]">
                {{ item.id === conversationAgentId ? '当前对话使用中' : formatUpdatedAt(item.updatedAt) }}
              </div>
            </button>

            <div
              v-if="bundles.length === 0"
              class="rounded-lg border border-dashed border-[#d8dee8] px-3 py-4 text-xs text-[#64748b] dark:border-[#3a3a3a] dark:text-[#a3a3a3]"
            >
              暂无套件，点击上方“新建套件”。
            </div>
          </div>
        </CardContent>
      </Card>

      <!-- 右：编辑器 -->
      <Card :class="panelClass">
        <CardContent v-if="!draft" class="flex h-full min-h-[440px] items-center justify-center px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">
          请选择或创建一个智能体套件。
        </CardContent>

        <div v-else class="flex h-full flex-col gap-4 px-3 pb-3">
          <!-- 基本信息 -->
          <div class="grid grid-cols-[160px_minmax(0,1fr)] gap-2">
            <Input v-model="draft.name" :class="fieldClass" placeholder="套件名称" />
            <Input v-model="draft.description" :class="fieldClass" placeholder="一句话描述（可选）" />
          </div>

          <!-- 提示词 -->
          <div class="space-y-1.5">
            <div class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">附加系统提示词</div>
            <Textarea
              v-model="draft.prompt"
              class="min-h-[130px] w-full resize-y border-[#d8dee8] bg-white font-mono text-[13px] leading-6 text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#202020] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]"
              spellcheck="false"
              placeholder="## Agent: xxx&#10;角色设定、行为约束、输出格式...（作为独立 section 追加到系统提示词）"
            />
          </div>

          <!-- 工具清单 -->
          <div class="space-y-1.5">
            <div class="flex items-center justify-between">
              <div class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">内置工具</div>
              <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
                <button
                  type="button"
                  class="px-2.5 py-1 transition-colors"
                  :class="!useCustomTools ? modeTabActiveClass : modeTabIdleClass"
                  @click="useCustomTools = false"
                >全部</button>
                <button
                  type="button"
                  class="border-l border-[#d8dee8] px-2.5 py-1 transition-colors dark:border-[#3a3a3a]"
                  :class="useCustomTools ? modeTabActiveClass : modeTabIdleClass"
                  @click="enableCustomMode('tools')"
                >自定义</button>
              </div>
            </div>
            <div v-if="!useCustomTools" class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">
              使用全部内置工具（自动包含后续新增的工具）。
            </div>
            <template v-else>
              <div class="flex items-center justify-between">
                <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">
                  已选 {{ toolSelection.size }} / {{ configurableTools.length }}（勾选 = 添加，取消 = 移除）
                </span>
                <div class="flex gap-1">
                  <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('tools', true)">全选</Button>
                  <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('tools', false)">清空</Button>
                </div>
              </div>
              <div class="grid max-h-[170px] grid-cols-2 gap-1 overflow-y-auto rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333] custom-scrollbar">
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

          <!-- 技能清单 -->
          <div class="space-y-1.5">
            <div class="flex items-center justify-between">
              <div class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">技能</div>
              <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
                <button
                  type="button"
                  class="px-2.5 py-1 transition-colors"
                  :class="!useCustomSkills ? modeTabActiveClass : modeTabIdleClass"
                  @click="useCustomSkills = false"
                >全部</button>
                <button
                  type="button"
                  class="border-l border-[#d8dee8] px-2.5 py-1 transition-colors dark:border-[#3a3a3a]"
                  :class="useCustomSkills ? modeTabActiveClass : modeTabIdleClass"
                  @click="enableCustomMode('skills')"
                >自定义</button>
              </div>
            </div>
            <div v-if="!useCustomSkills" class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">
              使用全部已启用技能（全局停用的技能始终不可见）。
            </div>
            <template v-else>
              <div class="flex items-center justify-between">
                <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">
                  已选 {{ skillSelection.size }} / {{ skills.length }}（勾选 = 添加，取消 = 移除）
                </span>
                <div class="flex gap-1">
                  <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('skills', true)">全选</Button>
                  <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('skills', false)">清空</Button>
                </div>
              </div>
              <div class="max-h-[120px] space-y-1 overflow-y-auto rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333] custom-scrollbar">
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

          <!-- MCP 清单 -->
          <div class="space-y-1.5">
            <div class="flex items-center justify-between">
              <div class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">MCP 服务器</div>
              <div class="flex overflow-hidden rounded-md border border-[#d8dee8] text-[12px] dark:border-[#3a3a3a]">
                <button
                  type="button"
                  class="px-2.5 py-1 transition-colors"
                  :class="!useCustomMcp ? modeTabActiveClass : modeTabIdleClass"
                  @click="useCustomMcp = false"
                >全部</button>
                <button
                  type="button"
                  class="border-l border-[#d8dee8] px-2.5 py-1 transition-colors dark:border-[#3a3a3a]"
                  :class="useCustomMcp ? modeTabActiveClass : modeTabIdleClass"
                  @click="enableCustomMode('mcp')"
                >自定义</button>
              </div>
            </div>
            <div v-if="!useCustomMcp" class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">
              接入全部已连接的 MCP 服务器。
            </div>
            <template v-else>
              <div class="flex items-center justify-between">
                <span class="text-[11.5px] text-[#64748b] dark:text-[#a3a3a3]">
                  已选 {{ mcpSelection.size }} / {{ mcpServers.length }}（勾选 = 接入，取消 = 断开）
                </span>
                <div class="flex gap-1">
                  <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('mcp', true)">全选</Button>
                  <Button variant="outline" size="sm" :class="headerButtonClass" @click="setAllSelection('mcp', false)">清空</Button>
                </div>
              </div>
              <div class="max-h-[120px] space-y-1 overflow-y-auto rounded-lg border border-[#e5e7eb] p-2 dark:border-[#333] custom-scrollbar">
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

          <!-- 底部操作 -->
          <div class="mt-auto flex flex-wrap items-center justify-between gap-2 border-t border-[#e5e7eb] pt-3 dark:border-[#333]">
            <div class="flex items-center gap-2">
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
            <div class="flex items-center gap-2">
              <span v-if="hasChanges" class="text-[12px] text-[#2563eb] dark:text-[#93c5fd]">有未保存改动</span>
              <Button variant="outline" size="sm" :class="headerButtonClass" :disabled="!hasChanges" @click="discardChanges">
                放弃改动
              </Button>
              <Button size="sm" :class="primaryButtonClass" :disabled="!hasChanges || saving" @click="saveBundle">
                {{ saving ? "保存中..." : "保存" }}
              </Button>
            </div>
          </div>
        </div>
      </Card>
    </div>
  </div>
</template>
