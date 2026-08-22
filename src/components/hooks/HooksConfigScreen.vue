<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { computed, onMounted, ref } from "vue";
import { emitToast } from "../../lib/toast";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

type MainView = "chat" | "hooks";

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
}>();

const loading = ref(false);
const saving = ref(false);
const content = ref("");
const savedContent = ref("");
const parseError = ref("");
const handlerCount = ref<number | null>(null);
const lastSavedAt = ref<number | null>(null);

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const headerButtonClass =
  "h-8 border border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const primaryButtonClass =
  "h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] focus-visible:ring-[#111827]/20 dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white";
const flowClass =
  "rounded-[8px] border border-[#e5e7eb] bg-[#f8fafc] px-3 py-2.5 font-mono text-[11px] leading-relaxed text-[#475569] dark:border-[#333] dark:bg-[#262626] dark:text-[#9ca3af]";
const flowLabelClass = "font-sans font-semibold text-[#374151] dark:text-[#d7d7d7]";
const flowArrowClass = "text-[#94a3b8] dark:text-[#6b7280]";
const editorClass =
  "min-h-[420px] w-full resize-y rounded-[8px] border border-[#d8dee8] bg-[#f8fafc] p-3 font-mono text-[12.5px] leading-relaxed text-[#111827] outline-none focus-visible:border-[#2563eb] focus-visible:ring-2 focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#1b1b1b] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]";

/** 默认模板：覆盖最常用的几类挂钩，用户按需增删。 */
const TEMPLATE = `# Nova 声明式挂钩配置（hooks.toml）
# 每个事件可配置多个匹配器分组（[[hooks.事件]]），分组内顺序执行处理器。
# 处理器类型：command（外部命令，退出码 0=通过 / 2=拦截）
#             context（注入上下文）、block（拦截工具调用）
#             stopWhen（输出命中即停）、stopOnError（失败即停）
#             maxAssistantMessages（消息数上限）、appendStopReason（附加结束原因）

# ── 示例：Bash 工具执行前跑自定义检查脚本 ──
# [[hooks.PreToolUse]]
# matcher = "bash"                       # 工具名匹配，支持 * 通配；缺省匹配全部
#   [[hooks.PreToolUse.hooks]]
#   type = "command"
#   command = "pwsh -NoProfile -File C:/nova/checks.ps1"
#   commandWindows = "pwsh -NoProfile -File C:/nova/checks.ps1"
#   timeoutSec = 30                      # 缺省 30 秒，超时按拦截处理
#   # stdin 收到事件 JSON；exit 0 通过（stdout 进上下文），exit 2 拦截（stderr 为原因）

# ── 示例：每次工具调用前注入提醒 ──
# [[hooks.PreToolUse]]
#   [[hooks.PreToolUse.hooks]]
#   type = "context"
#   text = "执行工具 {tool_name} 前请再次确认参数"

# ── 示例：工具输出出现 FATAL 立即停止续跑 ──
# [[hooks.PostToolUse]]
#   [[hooks.PostToolUse.hooks]]
#   type = "stopWhen"
#   pattern = "FATAL"

# ── 示例：回合结束前注入检查提醒（相同内容只生效一次） ──
# [[hooks.Stop]]
#   [[hooks.Stop.hooks]]
#   type = "context"
#   text = "在结束前请确认所有 todo 都已完成"

# ── 示例：限制单回合助手消息数 ──
# [[hooks.Stop]]
#   [[hooks.Stop.hooks]]
#   type = "maxAssistantMessages"
#   limit = 12
`;

/** 事件文档（侧栏说明）。 */
const eventDocs: { name: string; desc: string; matcher: boolean }[] = [
  { name: "SessionStart", desc: "会话第一轮发送前", matcher: false },
  { name: "UserPromptSubmit", desc: "每次用户发送消息、调用模型之前", matcher: false },
  { name: "PreToolUse", desc: "AI 决定调用工具、真正执行之前", matcher: true },
  { name: "PostToolUse", desc: "工具执行成功后", matcher: true },
  { name: "PostToolUseFailure", desc: "工具执行失败后", matcher: true },
  { name: "PreCompact", desc: "上下文压缩之前", matcher: false },
  { name: "PostCompact", desc: "上下文压缩完成之后", matcher: false },
  { name: "SubagentStart", desc: "Task 子智能体启动时", matcher: false },
  { name: "SubagentStop", desc: "子智能体结束、结果返回主循环前", matcher: false },
  { name: "Stop", desc: "AI 无工具调用、回合即将结束时", matcher: false },
  { name: "SessionEnd", desc: "回合正常完成、发送 stop 事件前", matcher: false },
  { name: "Error", desc: "回合以错误结束时", matcher: false },
];

const dirty = computed(() => content.value !== savedContent.value);

async function loadHooks() {
  loading.value = true;
  parseError.value = "";
  try {
    const raw = await invoke<string>("get_hooks_toml");
    content.value = raw ?? "";
    savedContent.value = content.value;
    handlerCount.value = null;
  } catch (err) {
    console.error("Failed to load hooks.toml:", err);
    parseError.value = String(err);
  } finally {
    loading.value = false;
  }
}

async function saveHooks() {
  saving.value = true;
  parseError.value = "";
  try {
    // 后端先做 TOML 解析校验，失败会返回带行列号的错误且不落盘。
    const count = await invoke<number>("save_hooks_toml", { content: content.value });
    savedContent.value = content.value;
    handlerCount.value = count;
    lastSavedAt.value = Date.now();
    window.dispatchEvent(new CustomEvent("settings-updated"));
    emitToast({
      variant: "success",
      source: "hooks",
      message: count > 0 ? `挂钩配置已保存，共 ${count} 个处理器。` : "挂钩配置已保存（当前无处理器）。",
    });
  } catch (err) {
    parseError.value = String(err);
    emitToast({
      variant: "error",
      source: "hooks",
      message: "保存失败：配置未通过校验，未写入文件。",
    });
  } finally {
    saving.value = false;
  }
}

function applyTemplate() {
  content.value = TEMPLATE;
  parseError.value = "";
}

function clearAll() {
  content.value = "";
  parseError.value = "";
}

const savedAtText = computed(() => {
  if (!lastSavedAt.value) return "";
  return `已保存: ${new Date(lastSavedAt.value).toLocaleTimeString()}`;
});

onMounted(() => {
  void loadHooks();
});
</script>

<template>
  <div :class="pageClass">
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">挂钩配置（hooks.toml）</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
          声明式挂钩：在生命周期事件上挂载命令执行、上下文注入与流程控制。保存前后端都会校验 TOML 语法。
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <Button variant="ghost" size="sm" :class="headerButtonClass" @click="emit('change-main-view', 'chat')">
          返回聊天
        </Button>
        <Button variant="ghost" size="sm" :class="headerButtonClass" :disabled="loading || saving" @click="loadHooks">
          刷新
        </Button>
        <Button variant="ghost" size="sm" :class="headerButtonClass" :disabled="loading || saving" @click="applyTemplate">
          插入模板
        </Button>
        <Button variant="ghost" size="sm" :class="headerButtonClass" :disabled="loading || saving" @click="clearAll">
          清空
        </Button>
        <Button
          size="sm"
          :class="primaryButtonClass"
          :disabled="loading || saving || !dirty"
          @click="saveHooks"
        >
          {{ saving ? "保存中..." : "保存配置" }}
        </Button>
      </div>
    </header>

    <!-- 完整调用时序 -->
    <div :class="flowClass">
      <div :class="flowLabelClass">完整调用时序</div>
      <div class="mt-1.5">
        <span :class="flowLabelClass">会话主线：</span>
        [SessionStart] <span :class="flowArrowClass">→</span> [UserPromptSubmit] <span :class="flowArrowClass">→</span> 模型循环 <span :class="flowArrowClass">→</span> [Stop] <span :class="flowArrowClass">→</span> [SessionEnd] <span :class="flowArrowClass">/</span> [Error]
      </div>
      <div class="mt-1">
        <span :class="flowLabelClass">模型循环内（每轮）：</span>
        模型调用 <span :class="flowArrowClass">→</span> AI 要调工具？
        <span class="text-[#2563eb] dark:text-[#60a5fa]">是</span> <span :class="flowArrowClass">→</span> [PreToolUse] <span :class="flowArrowClass">→</span> 工具执行 <span :class="flowArrowClass">→</span> [PostToolUse]<span class="text-[#94a3b8]">（成功）</span> / [PostToolUseFailure]<span class="text-[#94a3b8]">（失败）</span> <span :class="flowArrowClass">→</span> 回到模型调用
      </div>
      <div class="mt-1">
        <span :class="flowLabelClass">子智能体（Task 工具内）：</span>
        [SubagentStart] <span :class="flowArrowClass">→</span> 子智能体执行（独立循环） <span :class="flowArrowClass">→</span> [SubagentStop] <span :class="flowArrowClass">→</span> 返回主循环
      </div>
    </div>

    <div class="grid flex-1 grid-cols-1 gap-3 xl:grid-cols-[1fr_300px]">
      <!-- 编辑器 -->
      <Card :class="panelClass">
        <CardContent class="space-y-2 px-4 pb-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="text-[13px] font-medium text-[#374151] dark:text-[#d7d7d7]">
              hooks.toml
              <span v-if="handlerCount !== null" class="ml-2 text-[11px] font-normal text-[#7b8494] dark:text-[#9ca3af]">
                当前生效处理器：{{ handlerCount }}
              </span>
              <span v-if="dirty" class="ml-2 text-[11px] font-normal text-amber-600 dark:text-amber-400">● 未保存</span>
              <span v-if="savedAtText" class="ml-2 text-[11px] font-normal text-[#7b8494] dark:text-[#9ca3af]">{{ savedAtText }}</span>
            </span>
          </div>
          <textarea
            v-model="content"
            :class="editorClass"
            spellcheck="false"
            placeholder="hooks.toml 为空 = 无任何挂钩。点击「插入模板」查看示例语法。"
          ></textarea>
          <p v-if="parseError" class="whitespace-pre-wrap rounded-md border border-red-200 bg-red-50 px-3 py-2 font-mono text-[12px] leading-relaxed text-red-700 dark:border-red-900 dark:bg-red-950/40 dark:text-red-300">
            {{ parseError }}
          </p>
        </CardContent>
      </Card>

      <!-- 事件与处理器文档 -->
      <Card :class="panelClass">
        <CardContent class="space-y-3 px-4 pb-4 text-[12px] leading-relaxed text-[#475569] dark:text-[#9ca3af]">
          <div>
            <div :class="flowLabelClass">生命周期事件</div>
            <ul class="mt-1.5 space-y-1">
              <li v-for="doc in eventDocs" :key="doc.name" class="flex items-baseline gap-1.5">
                <code class="shrink-0 rounded bg-[#f1f5f9] px-1 py-0.5 font-mono text-[11px] text-[#1d4ed8] dark:bg-[#2a2a2a] dark:text-[#93c5fd]">
                  {{ doc.name }}
                </code>
                <span>{{ doc.desc }}</span>
                <span v-if="doc.matcher" class="shrink-0 text-[10px] text-[#94a3b8]" title="支持 matcher 工具名匹配">ⓜ</span>
              </li>
            </ul>
          </div>
          <div>
            <div :class="flowLabelClass">处理器类型</div>
            <ul class="mt-1.5 space-y-1">
              <li><code class="font-mono text-[11px]">command</code>：执行外部命令。事件 JSON 经 stdin 传入；退出码 0 = 通过（stdout 进上下文），2 = 拦截（stderr 为原因），其他非零忽略。支持 <code class="font-mono text-[11px]">commandWindows</code>、<code class="font-mono text-[11px]">timeoutSec</code>、<code class="font-mono text-[11px]">async</code>。</li>
              <li><code class="font-mono text-[11px]">context</code>：注入一条上下文消息；支持占位符 {tool_name} {conversation_id} {subagent_name} {stop_reason} {error}。</li>
              <li><code class="font-mono text-[11px]">block</code>：拒绝当前工具调用（填 reason）。</li>
              <li><code class="font-mono text-[11px]">stopWhen</code>：工具输出 / 最新助手文本包含 pattern 时终止续跑。</li>
              <li><code class="font-mono text-[11px]">stopOnError</code>：工具失败时终止续跑。</li>
              <li><code class="font-mono text-[11px]">maxAssistantMessages</code>：助手消息数超过 limit 时终止续跑。</li>
              <li><code class="font-mono text-[11px]">appendStopReason</code>：把文本附加到结束原因 / 错误信息末尾。</li>
            </ul>
          </div>
          <div>
            <div :class="flowLabelClass">matcher 语法</div>
            <p class="mt-1">仅工具类事件（ⓜ）生效：大小写不敏感，支持 <code class="font-mono text-[11px]">*</code> 通配，如 <code class="font-mono text-[11px]">bash</code>、<code class="font-mono text-[11px]">mcp_*</code>；缺省匹配全部工具。</p>
          </div>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
