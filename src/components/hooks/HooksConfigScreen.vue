<script setup lang="ts">
// 挂钩配置：表单卡片式。一个挂钩 = 一张卡片（触发事件 + 工具过滤 + 动作参数），
// 保存时前端生成 hooks.toml 交后端校验落盘，用户全程不接触 TOML 语法。
import { invoke } from "@tauri-apps/api/core";
import { onMounted, ref } from "vue";
import { emitToast } from "../../lib/toast";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Trash2 } from "lucide-vue-next";

type MainView = "chat" | "hooks";

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
}>();

const loading = ref(false);
const saving = ref(false);
const loadError = ref("");
const saveError = ref("");
let uid = 0;

// ---------------- 数据模型 ----------------

const EVENTS: { value: string; label: string; toolEvent: boolean }[] = [
  { value: "SessionStart", label: "会话开始（第一轮发送前）", toolEvent: false },
  { value: "UserPromptSubmit", label: "用户每次发送消息时", toolEvent: false },
  { value: "PreToolUse", label: "工具执行前", toolEvent: true },
  { value: "PostToolUse", label: "工具执行成功后", toolEvent: true },
  { value: "PostToolUseFailure", label: "工具执行失败后", toolEvent: true },
  { value: "PreCompact", label: "上下文压缩前", toolEvent: false },
  { value: "PostCompact", label: "上下文压缩后", toolEvent: false },
  { value: "SubagentStart", label: "子智能体启动时", toolEvent: false },
  { value: "SubagentStop", label: "子智能体结束时", toolEvent: false },
  { value: "Stop", label: "回合即将结束时", toolEvent: false },
  { value: "SessionEnd", label: "会话正常完成时", toolEvent: false },
  { value: "Error", label: "回合以错误结束时", toolEvent: false },
];

const HANDLERS: { value: string; label: string }[] = [
  { value: "context", label: "注入提醒消息" },
  { value: "block", label: "拦截工具调用" },
  { value: "command", label: "执行外部命令" },
  { value: "stopWhen", label: "输出含关键词时停止" },
  { value: "stopOnError", label: "工具失败时停止" },
  { value: "maxAssistantMessages", label: "限制消息数" },
  { value: "appendStopReason", label: "附加结束原因" },
];

interface HookCard {
  id: number;
  event: string;
  matcher: string; // 空 = 全部工具
  type: string;
  command: string;
  commandWindows: string;
  timeoutSec: string; // 空 = 默认 30 秒
  async: boolean;
  text: string; // context / appendStopReason
  reason: string; // block
  pattern: string; // stopWhen
  limit: string; // maxAssistantMessages
}

const newCard = (partial?: Partial<HookCard>): HookCard => ({
  id: ++uid,
  event: "UserPromptSubmit",
  matcher: "",
  type: "context",
  command: "",
  commandWindows: "",
  timeoutSec: "",
  async: false,
  text: "",
  reason: "",
  pattern: "",
  limit: "",
  ...partial,
});

const cards = ref<HookCard[]>([]);

const isToolEvent = (event: string) =>
  EVENTS.find((e) => e.value === event)?.toolEvent ?? false;

const eventLabel = (event: string) =>
  EVENTS.find((e) => e.value === event)?.label ?? event;

const handlerLabel = (type: string) =>
  HANDLERS.find((h) => h.value === type)?.label ?? type;

// ---------------- 加载：结构化 JSON → 卡片 ----------------

async function loadHooks() {
  loading.value = true;
  loadError.value = "";
  try {
    const data = await invoke<Record<string, unknown>>("get_hooks_structured");
    cards.value = parseStructured(data);
  } catch (err) {
    console.error("Failed to load hooks:", err);
    loadError.value = String(err);
  } finally {
    loading.value = false;
  }
}

/** 把后端 HooksFile JSON 展平成卡片列表（一个处理器一张卡）。 */
function parseStructured(data: Record<string, unknown>): HookCard[] {
  const hooksObj = (data.hooks ?? {}) as Record<string, unknown>;
  const out: HookCard[] = [];
  for (const event of EVENTS) {
    const groups = hooksObj[event.value];
    if (!Array.isArray(groups)) continue;
    for (const group of groups as Record<string, unknown>[]) {
      const matcher = typeof group.matcher === "string" ? group.matcher : "";
      const handlers = Array.isArray(group.hooks) ? (group.hooks as Record<string, unknown>[]) : [];
      for (const handler of handlers) {
        const type = typeof handler.type === "string" ? handler.type : "context";
        out.push(
          newCard({
            event: event.value,
            matcher,
            type,
            command: typeof handler.command === "string" ? handler.command : "",
            commandWindows: typeof handler.commandWindows === "string" ? handler.commandWindows : "",
            timeoutSec: handler.timeoutSec != null ? String(handler.timeoutSec) : "",
            async: handler.async === true,
            text: typeof handler.text === "string" ? handler.text : "",
            reason: typeof handler.reason === "string" ? handler.reason : "",
            pattern: typeof handler.pattern === "string" ? handler.pattern : "",
            limit: handler.limit != null ? String(handler.limit) : "",
          }),
        );
      }
    }
  }
  return out;
}

// ---------------- 保存：卡片 → TOML 文本 ----------------

/** JSON 字符串转义与 TOML 基础字符串兼容，直接复用。 */
const tomlStr = (value: string) => JSON.stringify(value);

function handlerToToml(card: HookCard): string {
  const rows: string[] = [`  type = ${tomlStr(card.type)}`];
  switch (card.type) {
    case "command":
      rows.push(`  command = ${tomlStr(card.command.trim())}`);
      if (card.commandWindows.trim()) rows.push(`  commandWindows = ${tomlStr(card.commandWindows.trim())}`);
      if (card.timeoutSec.trim()) rows.push(`  timeoutSec = ${Number(card.timeoutSec) || 30}`);
      if (card.async) rows.push("  async = true");
      break;
    case "context":
    case "appendStopReason":
      rows.push(`  text = ${tomlStr(card.text)}`);
      break;
    case "block":
      rows.push(`  reason = ${tomlStr(card.reason)}`);
      break;
    case "stopWhen":
      rows.push(`  pattern = ${tomlStr(card.pattern)}`);
      break;
    case "maxAssistantMessages":
      rows.push(`  limit = ${Number(card.limit) || 12}`);
      break;
  }
  return [`  [[hooks.${card.event}.hooks]]`, ...rows].join("\n");
}

/** 生成完整 hooks.toml：按 (事件, 工具过滤) 分组。 */
function buildToml(): string {
  const blocks: string[] = [];
  const seen = new Set<string>();
  for (const card of cards.value) {
    const key = `${card.event}\u0000${card.matcher.trim()}`;
    if (!seen.has(key)) {
      seen.add(key);
      const matcher = card.matcher.trim();
      const header = [`[[hooks.${card.event}]]`];
      if (matcher && isToolEvent(card.event)) header.push(`matcher = ${tomlStr(matcher)}`);
      blocks.push(header.join("\n"));
    }
    blocks.push(handlerToToml(card));
  }
  return blocks.join("\n\n") + "\n";
}

/** 保存前的必填校验，返回错误文案；通过返回空串。 */
function validateCards(): string {
  for (const [index, card] of cards.value.entries()) {
    const where = `第 ${index + 1} 个挂钩（${eventLabel(card.event)}）`;
    switch (card.type) {
      case "command":
        if (!card.command.trim()) return `${where}：外部命令不能为空`;
        break;
      case "context":
      case "appendStopReason":
        if (!card.text.trim()) return `${where}：消息文本不能为空`;
        break;
      case "block":
        if (!card.reason.trim()) return `${where}：拦截原因不能为空`;
        break;
      case "stopWhen":
        if (!card.pattern.trim()) return `${where}：关键词不能为空`;
        break;
      case "maxAssistantMessages":
        if (!(Number(card.limit) > 0)) return `${where}：消息数上限需为正数`;
        break;
    }
  }
  return "";
}

async function saveHooks() {
  saveError.value = "";
  const invalid = validateCards();
  if (invalid) {
    saveError.value = invalid;
    return;
  }
  saving.value = true;
  try {
    const toml = cards.value.length > 0 ? buildToml() : "";
    const count = await invoke<number>("save_hooks_toml", { content: toml });
    window.dispatchEvent(new CustomEvent("settings-updated"));
    emitToast({
      variant: "success",
      source: "hooks",
      message: count > 0 ? `已保存 ${count} 个挂钩。` : "已保存（当前无挂钩）。",
    });
  } catch (err) {
    saveError.value = String(err);
    emitToast({ variant: "error", source: "hooks", message: "保存失败，请检查配置。" });
  } finally {
    saving.value = false;
  }
}

// ---------------- 卡片操作 ----------------

function addCard(partial?: Partial<HookCard>) {
  cards.value.push(newCard(partial));
}

function removeCard(id: number) {
  cards.value = cards.value.filter((card) => card.id !== id);
}

/** 常见场景：一键新增一张预填卡片。 */
const PRESETS: { title: string; partial: Partial<HookCard> }[] = [
  {
    title: "工具调用前提醒确认参数",
    partial: { event: "PreToolUse", type: "context", text: "执行工具 {tool_name} 前请再次确认参数是否正确" },
  },
  {
    title: "输出出现关键词立即停止",
    partial: { event: "PostToolUse", type: "stopWhen", pattern: "FATAL" },
  },
  {
    title: "工具失败时停止回合",
    partial: { event: "PostToolUseFailure", type: "stopOnError" },
  },
  {
    title: "回合结束前检查清单",
    partial: { event: "Stop", type: "context", text: "在结束前请确认所有任务都已完成并验证" },
  },
  {
    title: "限制单回合消息数",
    partial: { event: "Stop", type: "maxAssistantMessages", limit: "12" },
  },
];

// ---------------- 底部执行流程图 ----------------

/** 主流程节点（按回合执行顺序），点击节点直接新建对应事件的挂钩卡片；
 * branches 为该节点向下拉出的分支事件。 */
const FLOW_NODES: { event: string; label: string; branches?: { event: string; label: string }[] }[] = [
  { event: "SessionStart", label: "会话开始" },
  {
    event: "UserPromptSubmit",
    label: "用户发送",
    // 上下文过长时在发起模型调用前自动压缩，属于发送后的旁路。
    branches: [
      { event: "PreCompact", label: "压缩前" },
      { event: "PostCompact", label: "压缩后" },
    ],
  },
  {
    event: "PreToolUse",
    label: "工具执行前",
    // Task 工具内的子智能体生命周期。
    branches: [
      { event: "SubagentStart", label: "子智能体启动" },
      { event: "SubagentStop", label: "子智能体结束" },
    ],
  },
  {
    event: "PostToolUse",
    label: "工具成功后",
    // 工具执行的另一结果：失败旁路。
    branches: [{ event: "PostToolUseFailure", label: "工具失败后" }],
  },
  { event: "Stop", label: "回合结束前" },
  { event: "SessionEnd", label: "会话完成", branches: [{ event: "Error", label: "错误结束" }] },
];

/** 某事件上已挂的挂钩数（流程图徽标用）。 */
function hookCount(event: string): number {
  return cards.value.filter((card) => card.event === event).length;
}

onMounted(() => {
  void loadHooks();
});
</script>

<template>
  <div class="box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]">
    <!-- 头部 -->
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">挂钩配置</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
          在 AI 的执行节点上挂动作：注入提醒、拦截工具、执行脚本、控制停止。每个挂钩一张卡片，保存即生效。
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          class="h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]"
          @click="emit('change-main-view', 'chat')"
        >
          返回聊天
        </Button>
        <Button
          variant="ghost"
          size="sm"
          class="h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]"
          :disabled="loading || saving"
          @click="loadHooks"
        >
          重新加载
        </Button>
        <Button
          size="sm"
          class="h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white"
          :disabled="loading || saving"
          @click="saveHooks"
        >
          {{ saving ? "保存中..." : "保存配置" }}
        </Button>
      </div>
    </header>

    <!-- 快捷场景 + 空白添加 -->
    <div class="flex flex-wrap items-center gap-2">
      <span class="text-[12px] text-[#8a94a3] dark:text-[#858585]">快速添加：</span>
      <button
        v-for="preset in PRESETS"
        :key="preset.title"
        type="button"
        class="rounded-full border border-[#e7e9ee] bg-[#fafbfc] px-3 py-1 text-[12px] text-[#475569] transition-colors hover:border-[#cbd5e1] hover:bg-[#f3f5f8] dark:border-[#343434] dark:bg-white/5 dark:text-[#ccc] dark:hover:border-[#4a4a4a] dark:hover:bg-white/10"
        @click="addCard(preset.partial)"
      >
        + {{ preset.title }}
      </button>
      <button
        type="button"
        class="rounded-full border border-dashed border-[#cbd5e1] px-3 py-1 text-[12px] text-[#8a94a3] transition-colors hover:border-[#94a3b8] hover:text-[#475569] dark:border-[#4a4a4a] dark:text-[#858585] dark:hover:border-[#64748b] dark:hover:text-[#ccc]"
        @click="addCard()"
      >
        + 空白挂钩
      </button>
    </div>

    <!-- 错误提示 -->
    <p v-if="loadError" class="whitespace-pre-wrap rounded-md border border-red-200 bg-red-50 px-3 py-2 font-mono text-[12px] leading-relaxed text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300">
      加载失败：{{ loadError }}
    </p>
    <p v-if="saveError" class="whitespace-pre-wrap rounded-md border border-red-200 bg-red-50 px-3 py-2 font-mono text-[12px] leading-relaxed text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300">
      {{ saveError }}
    </p>

    <!-- 空状态 -->
    <div
      v-if="!loading && cards.length === 0"
      class="rounded-xl border border-dashed border-[#d8dee8] px-4 py-10 text-center dark:border-[#3a3a3a]"
    >
      <p class="text-[13px] text-[#64748b] dark:text-[#a3a3a3]">还没有配置任何挂钩</p>
      <p class="mt-1 text-[12px] text-[#94a3b8] dark:text-[#737373]">点上面的快速添加，或「+ 空白挂钩」从头创建</p>
    </div>

    <!-- 挂钩卡片列表 -->
    <div class="grid grid-cols-1 gap-3 xl:grid-cols-2">
      <div
        v-for="(card, index) in cards"
        :key="card.id"
        class="rounded-xl border border-[#e7e9ee] bg-white p-3 dark:border-[#343434] dark:bg-[#242424]"
      >
        <!-- 卡片头：序号 + 摘要 + 删除 -->
        <div class="mb-2.5 flex items-center gap-2">
          <span class="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-[#eef2f7] text-[11px] font-semibold text-[#475569] dark:bg-white/10 dark:text-[#ccc]">
            {{ index + 1 }}
          </span>
          <span class="min-w-0 flex-1 truncate text-[12.5px] font-medium text-[#111827] dark:text-[#ececec]">
            {{ eventLabel(card.event) }} · {{ handlerLabel(card.type) }}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            class="h-7 w-7 shrink-0 rounded-md text-[#94a3b8] hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-950/40 dark:hover:text-red-400"
            title="删除该挂钩"
            @click="removeCard(card.id)"
          >
            <Trash2 class="h-3.5 w-3.5" />
          </Button>
        </div>

        <div class="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          <!-- 触发事件 -->
          <div class="space-y-1">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">什么时候触发</label>
            <Select v-model="card.event">
              <SelectTrigger class="h-8 w-full text-[12.5px]">
                <SelectValue placeholder="选择事件" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem v-for="event in EVENTS" :key="event.value" :value="event.value">
                  {{ event.label }}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          <!-- 工具过滤：仅工具类事件显示 -->
          <div v-if="isToolEvent(card.event)" class="space-y-1">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">限定工具（留空 = 全部）</label>
            <Input v-model="card.matcher" class="h-8 text-[12.5px]" placeholder="如 bash、mcp_*" />
          </div>

          <!-- 动作类型 -->
          <div class="space-y-1" :class="isToolEvent(card.event) ? '' : 'sm:col-span-1'">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">执行什么动作</label>
            <Select v-model="card.type">
              <SelectTrigger class="h-8 w-full text-[12.5px]">
                <SelectValue placeholder="选择动作" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem v-for="handler in HANDLERS" :key="handler.value" :value="handler.value">
                  {{ handler.label }}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          <!-- 动作参数：按类型动态渲染 -->
          <div v-if="card.type === 'context' || card.type === 'appendStopReason'" class="space-y-1 sm:col-span-2">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">
              {{ card.type === 'context' ? '注入的消息内容（支持 {tool_name} {conversation_id} 占位符）' : '附加到结束原因的文本' }}
            </label>
            <Textarea v-model="card.text" rows="2" class="resize-none text-[12.5px]" placeholder="要注入的文本…" />
          </div>

          <div v-else-if="card.type === 'block'" class="space-y-1 sm:col-span-2">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">拦截原因（会反馈给 AI）</label>
            <Input v-model="card.reason" class="h-8 text-[12.5px]" placeholder="如：禁止删除文件" />
          </div>

          <div v-else-if="card.type === 'stopWhen'" class="space-y-1 sm:col-span-2">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">命中即停止的关键词</label>
            <Input v-model="card.pattern" class="h-8 text-[12.5px]" placeholder="如 FATAL" />
          </div>

          <div v-else-if="card.type === 'maxAssistantMessages'" class="space-y-1">
            <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">消息数上限</label>
            <Input v-model="card.limit" type="number" min="1" class="h-8 text-[12.5px]" placeholder="12" />
          </div>

          <template v-else-if="card.type === 'command'">
            <div class="space-y-1 sm:col-span-2">
              <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">
                命令（事件 JSON 从 stdin 传入；退出码 0 通过 / 2 拦截）
              </label>
              <Input v-model="card.command" class="h-8 font-mono text-[12px]" placeholder="pwsh -NoProfile -File C:/nova/checks.ps1" />
            </div>
            <div class="space-y-1">
              <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">Windows 专用命令（可选）</label>
              <Input v-model="card.commandWindows" class="h-8 font-mono text-[12px]" placeholder="留空则用上面的命令" />
            </div>
            <div class="space-y-1">
              <label class="text-[11px] font-medium text-[#64748b] dark:text-[#9ca3af]">超时秒数（默认 30）</label>
              <Input v-model="card.timeoutSec" type="number" min="1" class="h-8 text-[12.5px]" placeholder="30" />
            </div>
          </template>
        </div>
      </div>
    </div>

    <!-- 执行流程图：展示挂钩挂载位置，圆点沿流程线流动；点击节点快速添加挂钩 -->
    <div class="rounded-xl border border-[#e7e9ee] bg-[#fafbfc] px-4 py-3.5 dark:border-[#343434] dark:bg-white/5">
      <div class="mb-3 flex items-center justify-between gap-2">
        <span class="text-[12.5px] font-medium text-[#374151] dark:text-[#d7d7d7]">执行流程</span>
        <span class="text-[11px] text-[#94a3b8] dark:text-[#737373]">点击节点快速添加挂钩，蓝色节点 = 已配置</span>
      </div>

      <!-- 主流程：节点 + 连接线，圆点依次流动；带分支的节点向下拉线挂接分支事件 -->
      <div class="flex items-start overflow-x-auto pb-1">
        <template v-for="(node, index) in FLOW_NODES" :key="node.event">
          <div v-if="index > 0" class="hook-flow-line mt-[14px]">
            <span class="hook-flow-dot" :style="{ animationDelay: `${index * 0.32}s` }" />
          </div>
          <div class="flex shrink-0 flex-col items-center">
            <button
              type="button"
              class="hook-flow-node"
              :class="{ 'is-hooked': hookCount(node.event) > 0 }"
              :title="`点击添加「${node.label}」挂钩`"
              @click="addCard({ event: node.event })"
            >
              {{ node.label }}
              <span v-if="hookCount(node.event) > 0" class="hook-flow-badge">{{ hookCount(node.event) }}</span>
            </button>

            <!-- 向下拉出的分支线 + 分支事件节点 -->
            <template v-if="node.branches && node.branches.length > 0">
              <div class="hook-flow-branch-line" />
              <template v-for="(branch, bIndex) in node.branches" :key="branch.event">
                <button
                  type="button"
                  class="hook-flow-node hook-flow-node-branch"
                  :class="{ 'is-hooked': hookCount(branch.event) > 0 }"
                  :title="`点击添加「${branch.label}」挂钩`"
                  @click="addCard({ event: branch.event })"
                >
                  {{ branch.label }}
                  <span v-if="hookCount(branch.event) > 0" class="hook-flow-badge">{{ hookCount(branch.event) }}</span>
                </button>
                <div v-if="bIndex < node.branches.length - 1" class="hook-flow-branch-line" />
              </template>
            </template>
          </div>
        </template>
      </div>
      <p class="mt-2 text-[11px] text-[#94a3b8] dark:text-[#737373]">竖线 = 该节点的旁路时机（压缩/工具失败/子智能体等），同样可点击添加挂钩</p>
    </div>
  </div>
</template>

<style scoped>
/* 流程连接线：细线 + 流动圆点 */
.hook-flow-line {
  position: relative;
  flex: 1 0 28px;
  height: 2px;
  min-width: 28px;
  border-radius: 999px;
  background: #e2e8f0;
}

.dark .hook-flow-line {
  background: #3f3f46;
}

.hook-flow-dot {
  position: absolute;
  top: -2px;
  left: 0;
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: #2563eb;
  opacity: 0;
  animation: hook-flow-move 1.9s linear infinite;
}

.dark .hook-flow-dot {
  background: #60a5fa;
}

@keyframes hook-flow-move {
  0% {
    left: 0;
    opacity: 0;
  }
  12% {
    opacity: 1;
  }
  85% {
    opacity: 1;
  }
  100% {
    left: calc(100% - 6px);
    opacity: 0;
  }
}

/* 流程节点 */
.hook-flow-node {
  position: relative;
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 4px;
  padding: 6px 12px;
  border: 1px solid #e7e9ee;
  border-radius: 10px;
  background: #fff;
  color: #475569;
  font-size: 12px;
  white-space: nowrap;
  cursor: pointer;
  transition: border-color 0.15s ease, background 0.15s ease, color 0.15s ease;
}

.hook-flow-node:hover {
  border-color: #93c5fd;
  background: #eff6ff;
  color: #1d4ed8;
}

.hook-flow-node.is-hooked {
  border-color: #93c5fd;
  background: #eff6ff;
  color: #1d4ed8;
}

.dark .hook-flow-node {
  border-color: #343434;
  background: #242424;
  color: #ccc;
}

.dark .hook-flow-node:hover,
.dark .hook-flow-node.is-hooked {
  border-color: #3b82f6;
  background: rgba(59, 130, 246, 0.14);
  color: #93c5fd;
}

/* 挂钩数徽标 */
.hook-flow-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 999px;
  background: #2563eb;
  color: #fff;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
}

.dark .hook-flow-badge {
  background: #60a5fa;
  color: #111;
}

/* 分支竖线：从主流程节点向下拉出 */
.hook-flow-branch-line {
  width: 2px;
  height: 12px;
  border-radius: 999px;
  background: #e2e8f0;
}

.dark .hook-flow-branch-line {
  background: #3f3f46;
}

/* 分支节点：比主节点略小，虚线边框区分 */
.hook-flow-node-branch {
  padding: 4px 10px;
  border-style: dashed;
  font-size: 11.5px;
}
</style>
