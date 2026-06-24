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
const labelClass = "text-[13px] text-[#374151] dark:text-[#d7d7d7]";
const hintClass = "text-[11px] text-[#7b8494] dark:text-[#9ca3af]";
const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
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
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">管理会话、提示提交、压缩前后、工具前后、子智能体、停止与错误等全流程 Hook。</p>
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

    <div v-else class="grid grid-cols-1 gap-3 xl:grid-cols-2">
      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">会话开始</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">上下文注入</Label>
          <Textarea v-model="form.sessionStartContext" :class="fieldClass" rows="3" placeholder="新的会话开始时追加的上下文" />
          <p :class="hintClass">对应 NOVA_SESSION_START_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">用户提示提交</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">上下文注入</Label>
          <Textarea v-model="form.userPromptSubmitContext" :class="fieldClass" rows="3" placeholder="每次用户提交提示时追加的上下文" />
          <p :class="hintClass">对应 NOVA_USER_PROMPT_SUBMIT_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">预压缩</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">上下文注入</Label>
          <Textarea v-model="form.preCompactContext" :class="fieldClass" rows="3" placeholder="压缩上下文前追加的提示" />
          <p :class="hintClass">对应 NOVA_PRE_COMPACT_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">后压缩</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">上下文注入</Label>
          <Textarea v-model="form.postCompactContext" :class="fieldClass" rows="3" placeholder="压缩完成后追加的提示" />
          <p :class="hintClass">对应 NOVA_POST_COMPACT_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PreToolUse</CardTitle>
        </CardHeader>
        <CardContent class="space-y-4 px-3">
          <div class="space-y-2">
            <Label :class="labelClass">禁用工具列表</Label>
            <Input v-model="form.preToolDenyTools" :class="fieldClass" placeholder="例如: Bash,Write" />
            <p :class="hintClass">对应 NOVA_PRE_TOOL_DENY_TOOLS，逗号分隔，名称按小写匹配。</p>
          </div>
          <div class="space-y-2">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.preToolContext" :class="fieldClass" rows="3" placeholder="进入工具执行前追加的提示内容" />
            <p :class="hintClass">对应 NOVA_PRE_TOOL_CONTEXT。</p>
          </div>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PostToolUse</CardTitle>
        </CardHeader>
        <CardContent class="space-y-4 px-3">
          <div class="space-y-2">
            <Label :class="labelClass">注入上下文</Label>
            <Textarea v-model="form.postToolContext" :class="fieldClass" rows="3" placeholder="工具执行后追加的提示内容" />
            <p :class="hintClass">对应 NOVA_POST_TOOL_CONTEXT。</p>
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
            <p :class="hintClass">对应 NOVA_POST_TOOL_STOP_ON_ERROR。</p>
          </div>

          <div class="space-y-2">
            <Label :class="labelClass">输出拦截关键字</Label>
            <Input v-model="form.postToolBlockPattern" :class="fieldClass" placeholder="命中该文本即停止续跑" />
            <p :class="hintClass">对应 NOVA_POST_TOOL_BLOCK_PATTERN。</p>
          </div>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">PostToolUseFailure</CardTitle>
        </CardHeader>
        <CardContent class="space-y-4 px-3">
          <div class="space-y-2">
            <Label :class="labelClass">失败上下文</Label>
            <Textarea v-model="form.postToolFailureContext" :class="fieldClass" rows="3" placeholder="工具失败后追加的提示内容" />
            <p :class="hintClass">对应 NOVA_POST_TOOL_FAILURE_CONTEXT。</p>
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
            <p :class="hintClass">对应 NOVA_POST_TOOL_FAILURE_STOP。</p>
          </div>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">StopHook</CardTitle>
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
            <p :class="hintClass">对应 NOVA_STOP_HOOK_MAX_ASSISTANT_MESSAGES，留空表示不启用限制。</p>
            <p v-if="stopHookMaxAssistantMessagesError" class="text-xs text-destructive">{{ stopHookMaxAssistantMessagesError }}</p>
          </div>

          <div class="space-y-2">
            <Label :class="labelClass">停止关键字</Label>
            <Input v-model="form.stopHookBlockPattern" :class="fieldClass" placeholder="命中 assistant 文本即终止" />
            <p :class="hintClass">对应 NOVA_STOP_HOOK_BLOCK_PATTERN。</p>
          </div>

          <div class="space-y-2">
            <Label :class="labelClass">附加上下文</Label>
            <Textarea v-model="form.stopHookAppendContext" :class="fieldClass" rows="3" placeholder="回合结束前追加的上下文" />
            <p :class="hintClass">对应 NOVA_STOP_HOOK_APPEND_CONTEXT。</p>
          </div>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">子智能体启动</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">上下文注入</Label>
          <Textarea v-model="form.subagentStartContext" :class="fieldClass" rows="3" placeholder="子智能体启动时追加上下文" />
          <p :class="hintClass">对应 NOVA_SUBAGENT_START_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">子智能体停止</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">上下文注入</Label>
          <Textarea v-model="form.subagentStopContext" :class="fieldClass" rows="3" placeholder="子智能体停止时追加上下文" />
          <p :class="hintClass">对应 NOVA_SUBAGENT_STOP_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">会话结束</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">结束原因附加文本</Label>
          <Textarea v-model="form.sessionEndContext" :class="fieldClass" rows="3" placeholder="会话结束时附加到 stop reason" />
          <p :class="hintClass">对应 NOVA_SESSION_END_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">出错</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 px-3">
          <Label :class="labelClass">错误附加文本</Label>
          <Textarea v-model="form.errorContext" :class="fieldClass" rows="3" placeholder="发生错误时附加到错误信息" />
          <p :class="hintClass">对应 NOVA_ERROR_HOOK_CONTEXT。</p>
        </CardContent>
      </Card>
    </div>

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
