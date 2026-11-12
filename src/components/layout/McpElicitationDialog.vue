<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface McpElicitationPayload {
  requestId: string;
  serverName: string;
  mode: "form" | "url";
  message: string;
  url?: string | null;
  requestedSchema?: {
    type?: string;
    properties?: Record<
      string,
      {
        type?: string;
        title?: string;
        description?: string;
        default?: unknown;
        enum?: unknown[];
      }
    >;
    required?: string[];
  } | null;
  conversationId?: string | null;
}

const queue = ref<McpElicitationPayload[]>([]);
const formValues = ref<Record<string, unknown>>({});
const submitting = ref(false);
const errorText = ref("");

const current = computed(() => queue.value[0] ?? null);

const schemaProperties = computed(() => {
  const props = current.value?.requestedSchema?.properties ?? {};
  return Object.entries(props).map(([key, schema]) => ({
    key,
    schema,
    required: current.value?.requestedSchema?.required?.includes(key) ?? false,
  }));
});

function initFormFor(payload: McpElicitationPayload) {
  const initial: Record<string, unknown> = {};
  const props = payload.requestedSchema?.properties ?? {};
  for (const [key, schema] of Object.entries(props)) {
    if (schema.default !== undefined) {
      initial[key] = schema.default;
    } else if (schema.type === "boolean") {
      initial[key] = false;
    } else {
      initial[key] = "";
    }
  }
  formValues.value = initial;
  errorText.value = "";
}

async function openExternalUrl() {
  const target = current.value?.url;
  if (!target) return;
  try {
    await openUrl(target);
  } catch {
    window.open(target, "_blank", "noopener,noreferrer");
  }
}

function buildSubmittedContent(): Record<string, unknown> | undefined {
  if (!current.value || current.value.mode !== "form") {
    return undefined;
  }
  const out: Record<string, unknown> = {};
  for (const { key, schema, required } of schemaProperties.value) {
    const raw = formValues.value[key];
    if (schema.type === "boolean") {
      out[key] = Boolean(raw);
      continue;
    }
    if (schema.type === "number" || schema.type === "integer") {
      const str = String(raw ?? "").trim();
      if (str === "") {
        if (required) throw new Error(`请填写必填字段：${schema.title || key}`);
        continue;
      }
      const parsed = schema.type === "integer" ? Number.parseInt(str, 10) : Number(str);
      if (Number.isNaN(parsed)) {
        throw new Error(`字段 ${schema.title || key} 需要输入有效数字`);
      }
      out[key] = parsed;
      continue;
    }
    const text = String(raw ?? "");
    if (required && text.trim() === "") {
      throw new Error(`请填写必填字段：${schema.title || key}`);
    }
    if (text !== "" || required) {
      out[key] = text;
    }
  }
  return out;
}

async function submitDecision(action: "accept" | "decline" | "cancel") {
  const active = current.value;
  if (!active || submitting.value) return;
  errorText.value = "";

  let content: Record<string, unknown> | undefined;
  if (action === "accept") {
    try {
      content = buildSubmittedContent();
    } catch (err) {
      errorText.value = err instanceof Error ? err.message : String(err);
      return;
    }
  }

  submitting.value = true;
  try {
    await invoke<boolean>("resolve_mcp_elicitation", {
      requestId: active.requestId,
      decision: {
        action,
        ...(content ? { content } : {}),
      },
    });
    queue.value.shift();
    if (queue.value[0]) {
      initFormFor(queue.value[0]);
    }
  } catch (err) {
    errorText.value = err instanceof Error ? err.message : String(err);
  } finally {
    submitting.value = false;
  }
}

let unlisten: UnlistenFn | null = null;

onMounted(async () => {
  try {
    unlisten = await listen<McpElicitationPayload>("mcp-elicitation-requested", (event) => {
      const payload = event.payload;
      queue.value.push(payload);
      if (queue.value.length === 1) {
        initFormFor(payload);
      }
    });
  } catch (err) {
    console.error("Failed to attach mcp-elicitation-requested listener:", err);
  }
});

onUnmounted(() => {
  if (unlisten) unlisten();
});
</script>

<template>
  <div
    v-if="current"
    class="fixed inset-0 z-[95] flex items-center justify-center bg-black/45 backdrop-blur-[1px] p-4"
  >
    <div
      class="w-full max-w-[480px] rounded-2xl border border-[#e5e7eb] dark:border-[#333] bg-white dark:bg-[#1e1e1e] shadow-2xl p-5 flex flex-col gap-4"
    >
      <div class="flex items-start justify-between gap-3">
        <div>
          <div class="text-[11px] font-mono uppercase tracking-wider text-blue-600 dark:text-blue-400">
            MCP 交互请求 · {{ current.serverName }}
          </div>
          <h3 class="text-[15px] font-semibold text-[#111827] dark:text-[#f3f4f6] mt-1">
            {{ current.mode === "url" ? "外部授权 / 页面确认" : "需要补充参数" }}
          </h3>
        </div>
        <span
          v-if="queue.length > 1"
          class="text-[11px] px-2 py-0.5 rounded-full bg-[#f3f4f6] dark:bg-[#2a2a2a] text-[#6b7280] dark:text-[#a3a3a3]"
        >
          1 / {{ queue.length }}
        </span>
      </div>

      <p class="text-[13.5px] leading-relaxed text-[#374151] dark:text-[#d4d4d4] whitespace-pre-wrap">
        {{ current.message }}
      </p>

      <div
        v-if="current.mode === 'url' && current.url"
        class="rounded-xl border border-[#e5e7eb] dark:border-[#333] bg-[#f9fafb] dark:bg-[#252525] p-3 flex flex-col gap-2"
      >
        <div class="text-[11.5px] text-[#6b7280] dark:text-[#a3a3a3]">目标地址（请确认信任后再打开）</div>
        <div class="text-[12.5px] font-mono break-all text-[#111827] dark:text-[#f3f4f6]">
          {{ current.url }}
        </div>
        <div class="flex justify-end">
          <Button variant="outline" size="sm" @click="openExternalUrl">在浏览器中打开</Button>
        </div>
      </div>

      <div v-else-if="current.mode === 'form'" class="flex flex-col gap-3 max-h-[300px] overflow-y-auto pr-1">
        <div v-for="field in schemaProperties" :key="field.key" class="flex flex-col gap-1.5">
          <label class="text-[12.5px] font-medium text-[#111827] dark:text-[#f3f4f6]">
            {{ field.schema.title || field.key }}
            <span v-if="field.required" class="text-red-500 ml-0.5">*</span>
          </label>
          <p v-if="field.schema.description" class="text-[11.5px] text-[#6b7280] dark:text-[#a3a3a3]">
            {{ field.schema.description }}
          </p>

          <label
            v-if="field.schema.type === 'boolean'"
            class="flex items-center gap-2 text-[13px] text-[#374151] dark:text-[#d4d4d4] cursor-pointer"
          >
            <input
              v-model="formValues[field.key]"
              type="checkbox"
              class="rounded border-[#d1d5db] dark:border-[#555]"
            />
            <span>启用</span>
          </label>

          <select
            v-else-if="Array.isArray(field.schema.enum) && field.schema.enum.length > 0"
            v-model="formValues[field.key]"
            class="h-9 rounded-md border border-input bg-background px-3 text-[13.5px]"
          >
            <option value="">请选择...</option>
            <option v-for="opt in field.schema.enum" :key="String(opt)" :value="opt">
              {{ String(opt) }}
            </option>
          </select>

          <Input
            v-else
            v-model="formValues[field.key] as string"
            :type="field.schema.type === 'number' || field.schema.type === 'integer' ? 'number' : 'text'"
            class="h-9 text-[13.5px]"
          />
        </div>
      </div>

      <div v-if="errorText" class="text-[12.5px] text-red-600 dark:text-red-400">
        {{ errorText }}
      </div>

      <div class="flex items-center justify-end gap-2 pt-1">
        <Button variant="ghost" size="sm" :disabled="submitting" @click="submitDecision('cancel')">
          取消
        </Button>
        <Button variant="outline" size="sm" :disabled="submitting" @click="submitDecision('decline')">
          拒绝
        </Button>
        <Button size="sm" :disabled="submitting" @click="submitDecision('accept')">
          {{ current.mode === "url" ? "确认已完成" : "提交" }}
        </Button>
      </div>
    </div>
  </div>
</template>
