<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { computed, onMounted, reactive, ref } from "vue";
import { emitToast } from "../../lib/toast";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

type MainView = "chat" | "hooks";

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
}>();

const loading = ref(false);
const saving = ref(false);
const lastSavedAt = ref<number | null>(null);

type HookGroup = "lifecycle" | "tool" | "subagent" | "stop";
const activeGroup = ref<HookGroup>("lifecycle");
const groups: { id: HookGroup; label: string; hint: string }[] = [
  { id: "lifecycle", label: "会话生命周期", hint: "SessionStart / UserPromptSubmit / PreCompact / PostCompact" },
  { id: "tool", label: "工具执行流程", hint: "PreToolUse / PostToolUse / PostToolUseFailure" },
  { id: "subagent", label: "子智能体", hint: "SubagentStart / SubagentStop" },
  { id: "stop", label: "停止与错误", hint: "StopHook / SessionEnd / Error" },
];

const form = reactive({
  sessionStartContext: "",
  userPromptSubmitContext: "",
  preCompactContext: "",
  postCompactContext: "",
  preToolDenyTools: "",
  preToolContext: "",
  postToolContext: "",
  postToolStopOnError: false,
  postToolBlockPattern: "",
  postToolFailureContext: "",
  postToolFailureStop: false,
  subagentStartContext: "",
  subagentStopContext: "",
  stopHookMaxAssistantMessages: "",
  stopHookBlockPattern: "",
  stopHookAppendContext: "",
  sessionEndContext: "",
  errorContext: "",
});

const fieldClass =
  "border-[#d8dee8] bg-white text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]";
const labelClass = "text-[13px] font-medium text-[#374151] dark:text-[#d7d7d7]";
const hintClass = "text-[11px] text-[#7b8494] dark:text-[#9ca3af]";
const exampleClass =
  "rounded-[6px] border border-dashed border-[#cbd5e1] bg-[#f8fafc] px-2 py-1.5 font-mono text-[11px] leading-relaxed text-[#475569] dark:border-[#3f3f46] dark:bg-[#262626] dark:text-[#9ca3af]";
const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const tabListClass =
  "flex flex-wrap items-center gap-1 rounded-[8px] border border-[#e5e7eb] bg-[#f8fafc] p-1 dark:border-[#333] dark:bg-[#262626]";
const tabItemClass =
  "h-7 rounded-[6px] px-3 text-[12px] font-medium transition-colors";
const tabItemActiveClass =
  "bg-white text-[#111827] shadow-sm dark:bg-[#404040] dark:text-[#f3f4f6]";
const tabItemIdleClass =
  "text-[#64748b] hover:text-[#111827] dark:text-[#9ca3af] dark:hover:text-[#f3f4f6]";
const tabHintClass =
  "mt-1.5 text-[11px] text-[#7b8494] dark:text-[#9ca3af]";
const flowClass =
  "rounded-[8px] border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2.5 font-mono text-[11px] leading-relaxed text-[#475569] dark:border-[#333] dark:bg-[#262626] dark:text-[#9ca3af]";
const flowLabelClass =
  "font-sans font-semibold text-[#374151] dark:text-[#d7d7d7]";
const flowArrowClass =
  "text-[#94a3b8] dark:text-[#6b7280]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const primaryButtonClass =
  "h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] focus-visible:ring-[#111827]/20 dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white";
const checkboxClass =
  "border-[#cbd5e1] shadow-none data-[state=checked]:border-[#2563eb] data-[state=checked]:bg-[#2563eb]";

function validateStopHookMaxAssistantMessages(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!/^\d+$/.test(trimmed)) {
    return "最大 Assistant 消息数仅支持非负整数。";
  }

  const parsed = Number.parseInt(trimmed, 10);
  if (!Number.isSafeInteger(parsed)) {
    return "数值过大，请输入较小的整数。";
  }

  return null;
}

function isTruthy(value: unknown): boolean {
  if (typeof value !== "string") return false;
  const normalized = value.trim().toLowerCase();
  return normalized === "1" || normalized === "true" || normalized === "yes" || normalized === "on";
}

function extractHookEnv(settings: Record<string, unknown>): Record<string, string> {
  const hookEnv = settings.hookEnv;
  if (hookEnv && typeof hookEnv === "object") {
    return hookEnv as Record<string, string>;
  }
  return {};
}

function applyHookEnvToForm(hookEnv: Record<string, string>) {
  form.sessionStartContext = hookEnv.NOVA_SESSION_START_HOOK_CONTEXT ?? "";
  form.userPromptSubmitContext = hookEnv.NOVA_USER_PROMPT_SUBMIT_HOOK_CONTEXT ?? "";
  form.preCompactContext = hookEnv.NOVA_PRE_COMPACT_HOOK_CONTEXT ?? "";
  form.postCompactContext = hookEnv.NOVA_POST_COMPACT_HOOK_CONTEXT ?? "";
  form.preToolDenyTools = hookEnv.NOVA_PRE_TOOL_DENY_TOOLS ?? "";
  form.preToolContext = hookEnv.NOVA_PRE_TOOL_CONTEXT ?? "";
  form.postToolContext = hookEnv.NOVA_POST_TOOL_CONTEXT ?? "";
  form.postToolStopOnError = isTruthy(hookEnv.NOVA_POST_TOOL_STOP_ON_ERROR);
  form.postToolBlockPattern = hookEnv.NOVA_POST_TOOL_BLOCK_PATTERN ?? "";
  form.postToolFailureContext = hookEnv.NOVA_POST_TOOL_FAILURE_CONTEXT ?? "";
  form.postToolFailureStop = isTruthy(hookEnv.NOVA_POST_TOOL_FAILURE_STOP);
  form.subagentStartContext = hookEnv.NOVA_SUBAGENT_START_HOOK_CONTEXT ?? "";
  form.subagentStopContext = hookEnv.NOVA_SUBAGENT_STOP_HOOK_CONTEXT ?? "";
  form.stopHookMaxAssistantMessages = hookEnv.NOVA_STOP_HOOK_MAX_ASSISTANT_MESSAGES ?? "";
  form.stopHookBlockPattern = hookEnv.NOVA_STOP_HOOK_BLOCK_PATTERN ?? "";
  form.stopHookAppendContext = hookEnv.NOVA_STOP_HOOK_APPEND_CONTEXT ?? "";
  form.sessionEndContext = hookEnv.NOVA_SESSION_END_HOOK_CONTEXT ?? "";
  form.errorContext = hookEnv.NOVA_ERROR_HOOK_CONTEXT ?? "";
}

function buildHookEnvFromForm(): Record<string, string> {
  const next: Record<string, string> = {};

  const put = (key: string, value: string) => {
    const trimmed = value.trim();
    if (trimmed) {
      next[key] = trimmed;
    }
  };

  put("NOVA_SESSION_START_HOOK_CONTEXT", form.sessionStartContext);
  put("NOVA_USER_PROMPT_SUBMIT_HOOK_CONTEXT", form.userPromptSubmitContext);
  put("NOVA_PRE_COMPACT_HOOK_CONTEXT", form.preCompactContext);
  put("NOVA_POST_COMPACT_HOOK_CONTEXT", form.postCompactContext);
  put("NOVA_PRE_TOOL_DENY_TOOLS", form.preToolDenyTools);
  put("NOVA_PRE_TOOL_CONTEXT", form.preToolContext);
  put("NOVA_POST_TOOL_CONTEXT", form.postToolContext);
  if (form.postToolStopOnError) {
    next.NOVA_POST_TOOL_STOP_ON_ERROR = "true";
  }
  put("NOVA_POST_TOOL_BLOCK_PATTERN", form.postToolBlockPattern);
  put("NOVA_POST_TOOL_FAILURE_CONTEXT", form.postToolFailureContext);
  if (form.postToolFailureStop) {
    next.NOVA_POST_TOOL_FAILURE_STOP = "true";
  }
  put("NOVA_SUBAGENT_START_HOOK_CONTEXT", form.subagentStartContext);
  put("NOVA_SUBAGENT_STOP_HOOK_CONTEXT", form.subagentStopContext);
  put("NOVA_STOP_HOOK_MAX_ASSISTANT_MESSAGES", form.stopHookMaxAssistantMessages);
  put("NOVA_STOP_HOOK_BLOCK_PATTERN", form.stopHookBlockPattern);
  put("NOVA_STOP_HOOK_APPEND_CONTEXT", form.stopHookAppendContext);
  put("NOVA_SESSION_END_HOOK_CONTEXT", form.sessionEndContext);
  put("NOVA_ERROR_HOOK_CONTEXT", form.errorContext);

  return next;
}

async function loadHookConfig() {
  loading.value = true;
  try {
    const settings = (await invoke("get_settings")) as Record<string, unknown>;
    const hookEnv = extractHookEnv(settings ?? {});
    applyHookEnvToForm(hookEnv);
  } catch (err) {
    console.error("Failed to load hook config:", err);
  } finally {
    loading.value = false;
  }
}

async function saveHookConfig() {
  const validationError = stopHookMaxAssistantMessagesError.value;
  if (validationError) {
    emitToast({
      variant: "error",
      source: "hooks",
      message: validationError,
    });
    return;
  }

  saving.value = true;
  try {
    const settings = (await invoke("get_settings")) as Record<string, unknown>;
    const nextSettings = {
      ...(settings ?? {}),
      hookEnv: buildHookEnvFromForm(),
    };
    await invoke("save_settings", { settings: nextSettings });

    lastSavedAt.value = Date.now();
    emitToast({
      variant: "success",
      source: "hooks",
      message: "钩子配置已保存并生效。",
    });
  } catch (err) {
    console.error("Failed to save hook config:", err);
  } finally {
    saving.value = false;
  }
}

function resetHookConfig() {
  applyHookEnvToForm({});
}

function onPostToolStopOnErrorChange(value: boolean | "indeterminate") {
  form.postToolStopOnError = value === true;
}

function onPostToolFailureStopChange(value: boolean | "indeterminate") {
  form.postToolFailureStop = value === true;
}

const savedAtText = computed(() => {
  if (!lastSavedAt.value) return "";
  return `已保存: ${new Date(lastSavedAt.value).toLocaleTimeString()}`;
});

const stopHookMaxAssistantMessagesError = computed(() =>
  validateStopHookMaxAssistantMessages(form.stopHookMaxAssistantMessages),
);

onMounted(() => {
  loadHookConfig();
});
</script>

<template>
  <div :class="pageClass">
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">挂钩配置</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">在会话生命周期的关键节点注入上下文或控制续跑行为。每个字段都标注了触发时机与最终注入到模型的内容。</p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          :class="headerButtonClass"
          @click="emit('change-main-view', 'chat')"
        >
          返回聊天
        </Button>
        <Button
          variant="ghost"
          size="sm"
          :class="headerButtonClass"
          :disabled="loading || saving"
          @click="loadHookConfig"
        >
          刷新
        </Button>
        <Button
          size="sm"
          :class="primaryButtonClass"
          :disabled="loading || saving || !!stopHookMaxAssistantMessagesError"
          @click="saveHookConfig"
        >
          {{ saving ? '保存中...' : '保存配置' }}
        </Button>
      </div>
    </header>

    <Card v-if="loading" :class="panelClass">
      <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取配置...</CardContent>
    </Card>

    <template v-else>
      <!-- 顶部 Tab 切换 -->
      <div :class="tabListClass">
        <button
          v-for="g in groups"
          :key="g.id"
          type="button"
          :class="[tabItemClass, activeGroup === g.id ? tabItemActiveClass : tabItemIdleClass]"
          @click="activeGroup = g.id"
        >
          {{ g.label }}
        </button>
      </div>
      <p :class="tabHintClass">{{ groups.find(g => g.id === activeGroup)?.hint }}</p>

      <!-- 完整调用时序 -->
      <div :class="flowClass">
        <div :class="flowLabelClass">完整调用时序</div>
        <div class="mt-1.5">
          <span :class="flowLabelClass">会话主线：</span>
          [SessionStart] <span :class="flowArrowClass">→</span> [UserPromptSubmit] <span :class="flowArrowClass">→</span> 模型循环 <span :class="flowArrowClass">→</span> [StopHook] <span :class="flowArrowClass">→</span> [SessionEnd] <span :class="flowArrowClass">/</span> [Error]
        </div>
        <div class="mt-1">
          <span :class="flowLabelClass">模型循环内（每轮）：</span>
          模型调用 <span :class="flowArrowClass">→</span> AI 要调工具？
          <span class="text-[#2563eb] dark:text-[#60a5fa]">是</span> <span :class="flowArrowClass">→</span> [PreToolUse] <span :class="flowArrowClass">→</span> 工具执行 <span :class="flowArrowClass">→</span> [PostToolUse]<span class="text-[#94a3b8]">（成功）</span> <span :class="flowArrowClass">/</span> [PostToolUseFailure]<span class="text-[#94a3b8]">（失败）</span> <span :class="flowArrowClass">→</span> 回到模型调用
        </div>
        <div class="mt-1">
          <span class="text-[#2563eb] dark:text-[#60a5fa]">否</span> <span :class="flowArrowClass">→</span> 上下文超阈值？
          <span class="text-[#2563eb] dark:text-[#60a5fa]">是</span> <span :class="flowArrowClass">→</span> [PreCompact] <span :class="flowArrowClass">→</span> 压缩 <span :class="flowArrowClass">→</span> [PostCompact] <span :class="flowArrowClass">→</span> 回到模型调用
          <span class="text-[#2563eb] dark:text-[#60a5fa]">否</span> <span :class="flowArrowClass">→</span> 回合结束，进入 [StopHook]
        </div>
        <div class="mt-1">
          <span :class="flowLabelClass">子智能体（Task 工具内）：</span>
          [SubagentStart] <span :class="flowArrowClass">→</span> 子智能体执行（独立循环） <span :class="flowArrowClass">→</span> [SubagentStop] <span :class="flowArrowClass">→</span> 返回主循环
        </div>
      </div>

      <!-- 分组 1：会话生命周期 -->
      <div v-if="activeGroup === 'lifecycle'" class="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">SessionStart · 会话开始</CardTitle>
            <p :class="hintClass">触发时机：用户开启新会话、进入空会话首次发送前。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.sessionStartContext" :class="fieldClass" rows="3" placeholder="例如：你是一个 Rust 专家，请优先使用 idiomatic 写法" />
            <p :class="hintClass">对应 NOVA_SESSION_START_HOOK_CONTEXT。留空表示不注入。</p>
            <div :class="exampleClass">→ user: [SessionStart] 你是一个 Rust 专家…</div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">UserPromptSubmit · 用户提交</CardTitle>
            <p :class="hintClass">触发时机：每次用户发送一条新消息时（在调用模型之前）。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.userPromptSubmitContext" :class="fieldClass" rows="3" placeholder="例如：回答前请先复述用户问题" />
            <p :class="hintClass">对应 NOVA_USER_PROMPT_SUBMIT_HOOK_CONTEXT。</p>
            <div :class="exampleClass">→ user: [UserPromptSubmit] 回答前请先复述用户问题</div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PreCompact · 压缩前</CardTitle>
            <p :class="hintClass">触发时机：上下文接近阈值、即将调用模型做摘要压缩之前。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.preCompactContext" :class="fieldClass" rows="3" placeholder="例如：压缩时请保留所有错误修复相关内容" />
            <p :class="hintClass">对应 NOVA_PRE_COMPACT_HOOK_CONTEXT。</p>
            <div :class="exampleClass">→ user: [PreCompact] 压缩时请保留所有错误修复相关内容</div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PostCompact · 压缩后</CardTitle>
            <p :class="hintClass">触发时机：上下文压缩完成、下一轮模型调用之前。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.postCompactContext" :class="fieldClass" rows="3" placeholder="例如：刚刚压缩过上下文，请确认你仍记得当前任务目标" />
            <p :class="hintClass">对应 NOVA_POST_COMPACT_HOOK_CONTEXT。</p>
            <div :class="exampleClass">→ user: [PostCompact] 刚刚压缩过上下文…</div>
          </CardContent>
        </Card>
      </div>

      <!-- 分组 2：工具执行流程 -->
      <div v-else-if="activeGroup === 'tool'" class="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PreToolUse · 工具调用前</CardTitle>
            <p :class="hintClass">触发时机：AI 决定调用工具、工具真正执行之前。</p>
          </CardHeader>
          <CardContent class="space-y-4 px-3">
            <div class="space-y-2">
              <Label :class="labelClass">禁用工具列表</Label>
              <Input v-model="form.preToolDenyTools" :class="fieldClass" placeholder="例如: Bash,Write" />
              <p :class="hintClass">对应 NOVA_PRE_TOOL_DENY_TOOLS，逗号分隔，名称按小写匹配。命中即报错终止本次工具调用。</p>
              <div :class="exampleClass">→ 工具调用被拒绝：Blocked by PreToolUse hook: tool 'Bash' is deny-listed</div>
            </div>
            <div class="space-y-2">
              <Label :class="labelClass">注入上下文</Label>
              <Textarea v-model="form.preToolContext" :class="fieldClass" rows="3" placeholder="例如：执行工具前请再次确认参数" />
              <p :class="hintClass">对应 NOVA_PRE_TOOL_CONTEXT。注入后会作为附加上下文进入下一轮。</p>
              <div :class="exampleClass">→ user: [PreToolUse] 执行工具前请再次确认参数</div>
            </div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PostToolUse · 工具调用后</CardTitle>
            <p :class="hintClass">触发时机：单个工具执行完成、产出结果之后。</p>
          </CardHeader>
          <CardContent class="space-y-4 px-3">
            <div class="space-y-2">
              <Label :class="labelClass">注入上下文</Label>
              <Textarea v-model="form.postToolContext" :class="fieldClass" rows="3" placeholder="例如：工具执行完毕，请检查输出是否符合预期" />
              <p :class="hintClass">对应 NOVA_POST_TOOL_CONTEXT。注入后会作为附加上下文进入下一轮。</p>
              <div :class="exampleClass">→ user: [PostToolUse] 工具执行完毕…</div>
            </div>

            <div class="space-y-2">
              <Label :class="labelClass">输出拦截关键字</Label>
              <Input v-model="form.postToolBlockPattern" :class="fieldClass" placeholder="命中该文本即停止续跑" />
              <p :class="hintClass">对应 NOVA_POST_TOOL_BLOCK_PATTERN。工具输出包含此关键字时，立即终止本轮续跑。</p>
              <div :class="exampleClass">→ 命中后：PostToolUse hook stopped continuation…</div>
            </div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PostToolUseFailure · 工具调用失败</CardTitle>
            <p :class="hintClass">触发时机：单个工具执行抛出错误或返回失败结果时。</p>
          </CardHeader>
          <CardContent class="space-y-4 px-3">
            <div class="space-y-2">
              <Label :class="labelClass">失败上下文</Label>
              <Textarea v-model="form.postToolFailureContext" :class="fieldClass" rows="3" placeholder="例如：工具失败，请尝试替代方案" />
              <p :class="hintClass">对应 NOVA_POST_TOOL_FAILURE_CONTEXT。注入后会作为附加上下文进入下一轮。</p>
              <div :class="exampleClass">→ user: [PostToolUseFailure] 工具失败，请尝试替代方案</div>
            </div>

            <div class="space-y-2">
              <div class="flex items-center gap-2">
                <Checkbox
                  id="post-tool-stop-on-error"
                  :class="checkboxClass"
                  :model-value="form.postToolStopOnError"
                  @update:model-value="onPostToolStopOnErrorChange"
                />
                <Label for="post-tool-stop-on-error" class="text-[13px] font-normal text-[#374151] dark:text-[#d7d7d7]">工具报错时终止续跑</Label>
              </div>
              <p :class="hintClass">对应 NOVA_POST_TOOL_STOP_ON_ERROR。等价于下面的"失败后直接终止续跑"，保留以兼容旧配置。</p>
            </div>

            <div class="space-y-2">
              <div class="flex items-center gap-2">
                <Checkbox
                  id="post-tool-failure-stop"
                  :class="checkboxClass"
                  :model-value="form.postToolFailureStop"
                  @update:model-value="onPostToolFailureStopChange"
                />
                <Label for="post-tool-failure-stop" class="text-[13px] font-normal text-[#374151] dark:text-[#d7d7d7]">失败后直接终止续跑</Label>
              </div>
              <p :class="hintClass">对应 NOVA_POST_TOOL_FAILURE_STOP。勾选后工具失败立即结束回合，不再续跑。</p>
              <div :class="exampleClass">→ 命中后：PostToolUseFailure hook stopped continuation after 'Bash' failed</div>
            </div>
          </CardContent>
        </Card>
      </div>

      <!-- 分组 3：子智能体 -->
      <div v-else-if="activeGroup === 'subagent'" class="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">SubagentStart · 子智能体启动</CardTitle>
            <p :class="hintClass">触发时机：Task 工具启动子智能体时。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.subagentStartContext" :class="fieldClass" rows="3" placeholder="例如：子智能体请在完成后输出简洁总结" />
            <p :class="hintClass">对应 NOVA_SUBAGENT_START_HOOK_CONTEXT。注入内容会附带子智能体名称。</p>
            <div :class="exampleClass">→ user: [SubagentStart] 子智能体请在完成后输出简洁总结 (name: search)</div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">SubagentStop · 子智能体停止</CardTitle>
            <p :class="hintClass">触发时机：子智能体执行完成、返回结果给主智能体之前。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.subagentStopContext" :class="fieldClass" rows="3" placeholder="例如：子智能体结果已返回，请综合判断" />
            <p :class="hintClass">对应 NOVA_SUBAGENT_STOP_HOOK_CONTEXT。注入内容会附带子智能体名称。</p>
            <div :class="exampleClass">→ user: [SubagentStop] 子智能体结果已返回… (name: search)</div>
          </CardContent>
        </Card>
      </div>

      <!-- 分组 4：停止与错误 -->
      <div v-else-if="activeGroup === 'stop'" class="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">StopHook · 回合停止</CardTitle>
            <p :class="hintClass">触发时机：AI 本轮无工具调用、回合即将结束时。</p>
          </CardHeader>
          <CardContent class="space-y-4 px-3">
            <div class="space-y-2">
              <Label :class="labelClass">最大 Assistant 消息数</Label>
              <Input
                v-model="form.stopHookMaxAssistantMessages"
                inputmode="numeric"
                placeholder="例如: 12"
                :class="[
                  fieldClass,
                  stopHookMaxAssistantMessagesError
                    ? 'border-destructive focus-visible:ring-destructive/20 dark:focus-visible:ring-destructive/40'
                    : '',
                ]"
              />
              <p :class="hintClass">对应 NOVA_STOP_HOOK_MAX_ASSISTANT_MESSAGES。当前会话累计 Assistant 消息数超过此值时立即终止续跑。留空表示不启用。</p>
              <p v-if="stopHookMaxAssistantMessagesError" class="text-xs text-destructive">{{ stopHookMaxAssistantMessagesError }}</p>
              <div :class="exampleClass">→ 超限后：Stop hook prevented continuation: assistant message count 13 exceeds limit 12</div>
            </div>

            <div class="space-y-2">
              <Label :class="labelClass">停止关键字</Label>
              <Input v-model="form.stopHookBlockPattern" :class="fieldClass" placeholder="命中 assistant 文本即终止" />
              <p :class="hintClass">对应 NOVA_STOP_HOOK_BLOCK_PATTERN。AI 最新输出包含此关键字时立即结束回合。</p>
              <div :class="exampleClass">→ 命中后：Stop hook prevented continuation because assistant text matched pattern 'DONE'</div>
            </div>

            <div class="space-y-2">
              <Label :class="labelClass">附加上下文</Label>
              <Textarea v-model="form.stopHookAppendContext" :class="fieldClass" rows="3" placeholder="例如：在结束前请确认所有 todo 都已完成" />
              <p :class="hintClass">对应 NOVA_STOP_HOOK_APPEND_CONTEXT。回合结束前注入一条提醒，模型会再跑一轮检查。仅当历史中不存在完全相同的字符串时才注入，因此静态内容最多触发一次续跑。</p>
              <div :class="exampleClass">→ user: [StopHookContext] 在结束前请确认所有 todo 都已完成</div>
            </div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">SessionEnd · 会话结束</CardTitle>
            <p :class="hintClass">触发时机：回合正常完成、向 UI 发送 stop 事件之前。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">结束原因附加文本</Label>
            <Textarea v-model="form.sessionEndContext" :class="fieldClass" rows="3" placeholder="例如：本回合由 Nova 终止" />
            <p :class="hintClass">对应 NOVA_SESSION_END_HOOK_CONTEXT。附加到 stop_reason 字符串末尾，不改变状态、不注入新消息。</p>
            <div :class="exampleClass">→ stop_reason: end_turn | [SessionEnd] 本回合由 Nova 终止</div>
          </CardContent>
        </Card>

        <Card :class="panelClass">
          <CardHeader class="px-3 pb-0">
            <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">Error · 错误处理</CardTitle>
            <p :class="hintClass">触发时机：回合以错误结束时（模型调用失败、工具链异常等）。</p>
          </CardHeader>
          <CardContent class="space-y-2 px-3">
            <Label :class="labelClass">错误附加文本</Label>
            <Textarea v-model="form.errorContext" :class="fieldClass" rows="3" placeholder="例如：如反复出错请提示用户检查 API Key" />
            <p :class="hintClass">对应 NOVA_ERROR_HOOK_CONTEXT。附加到错误信息末尾，作为 override_error 返回给前端。</p>
            <div :class="exampleClass">→ error: 请求失败 | [ErrorHook] 如反复出错请提示用户检查 API Key</div>
          </CardContent>
        </Card>
      </div>
    </template>

    <footer class="flex flex-wrap items-center justify-between gap-2">
      <Button
        variant="outline"
        size="sm"
        :class="headerButtonClass"
        :disabled="loading || saving"
        @click="resetHookConfig"
      >
        清空表单
      </Button>
      <span class="text-xs text-[#7b8494] dark:text-[#9ca3af]">{{ savedAtText }}</span>
    </footer>
  </div>
</template>
