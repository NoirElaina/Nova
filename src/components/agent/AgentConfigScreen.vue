<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { emitToast } from "@/lib/toast";

type MainView = "chat" | "hooks" | "agent";

type AgentProfileMeta = {
  id: string;
  name: string;
  fileName: string;
  updatedAt: number;
  path: string;
};

const emit = defineEmits<{
  (e: "change-main-view", view: MainView): void;
}>();

const loadingList = ref(false);
const loadingContent = ref(false);
const saving = ref(false);
const creating = ref(false);
const deleting = ref(false);
const showCreatePanel = ref(false);
const showDeletePanel = ref(false);
const newProfileName = ref("new-agent");
const profiles = ref<AgentProfileMeta[]>([]);
const selectedProfileId = ref("");
const selectedProfilePath = ref("");
const content = ref("");
const originalContent = ref("");

const pageClass =
  "box-border flex h-full flex-col gap-3 overflow-auto bg-white px-4 pb-4 pt-16 dark:bg-[#1e1e1e]";
const panelClass =
  "gap-3 border-[#e5e7eb] bg-white py-3 shadow-none dark:border-[#333] dark:bg-[#242424]";
const fieldClass =
  "border-[#d8dee8] bg-white text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]";
const headerButtonClass =
  "h-8 border-[#d8dee8] bg-white px-3 text-[13px] text-[#475569] shadow-none hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]";
const primaryButtonClass =
  "h-8 bg-[#111827] px-3 text-[13px] text-white shadow-none hover:bg-[#1f2937] focus-visible:ring-[#111827]/20 dark:bg-[#ededed] dark:text-[#111] dark:hover:bg-white";

const hasChanges = computed(() => content.value !== originalContent.value);
const hasSelectedProfile = computed(() => selectedProfileId.value.trim().length > 0);
const hasProfiles = computed(() => profiles.value.length > 0);
const isBusy = computed(
  () => loadingList.value || loadingContent.value || saving.value || creating.value || deleting.value,
);

const selectedProfile = computed(() =>
  profiles.value.find((item) => item.id === selectedProfileId.value) ?? null,
);

const selectedProfileLabel = computed(() => selectedProfile.value?.name ?? "未选择智能体");

const formatUpdatedAt = (unixSeconds: number) => {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) {
    return "--";
  }
  return new Date(unixSeconds * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
};

async function loadAgentProfiles() {
  loadingList.value = true;
  try {
    const items = await invoke<AgentProfileMeta[]>("list_agent_profiles");
    profiles.value = items ?? [];

    if (profiles.value.length === 0) {
      selectedProfileId.value = "";
      selectedProfilePath.value = "";
      content.value = "";
      originalContent.value = "";
      return;
    }

    const exists = profiles.value.some((item) => item.id === selectedProfileId.value);
    if (!exists) {
      selectedProfileId.value = profiles.value[0].id;
    }

    await loadSelectedProfileContent();
  } catch (err) {
    console.error("Failed to load agent profiles:", err);
  } finally {
    loadingList.value = false;
  }
}

async function loadSelectedProfileContent() {
  if (!selectedProfileId.value) {
    selectedProfilePath.value = "";
    content.value = "";
    originalContent.value = "";
    return;
  }

  loadingContent.value = true;
  try {
    const text = await invoke<string>("load_agent_profile_markdown", {
      profileId: selectedProfileId.value,
    });

    const matched = selectedProfile.value;
    selectedProfilePath.value = matched?.path ?? "";
    content.value = text ?? "";
    originalContent.value = content.value;
  } catch (err) {
    console.error("Failed to load agent profile markdown:", err);
  } finally {
    loadingContent.value = false;
  }
}

function openCreatePanel() {
  if (isBusy.value) {
    return;
  }
  showDeletePanel.value = false;
  newProfileName.value = "new-agent";
  showCreatePanel.value = true;
}

function cancelCreatePanel() {
  if (creating.value) {
    return;
  }
  showCreatePanel.value = false;
}

function openDeletePanel() {
  if (isBusy.value || !hasSelectedProfile.value) {
    return;
  }

  if (hasChanges.value) {
    emitToast({
      variant: "error",
      source: "agent-config",
      message: "当前有未保存内容，请先保存或撤销改动后再删除。",
    });
    return;
  }

  showCreatePanel.value = false;
  showDeletePanel.value = true;
}

function cancelDeletePanel() {
  if (deleting.value) {
    return;
  }
  showDeletePanel.value = false;
}

async function createAgentProfile() {
  if (creating.value) {
    return;
  }

  const nextName = newProfileName.value.trim();
  if (!nextName) {
    emitToast({
      variant: "error",
      source: "agent-config",
      message: "请输入智能体名称。",
    });
    return;
  }

  creating.value = true;
  try {
    const created = await invoke<AgentProfileMeta>("create_agent_profile", {
      name: nextName,
    });

    await loadAgentProfiles();
    if (created?.id) {
      selectedProfileId.value = created.id;
      await loadSelectedProfileContent();
    }

    showCreatePanel.value = false;
    emitToast({
      variant: "success",
      source: "agent-config",
      message: "已创建智能体。",
    });
  } catch (err) {
    console.error("Failed to create agent profile:", err);
  } finally {
    creating.value = false;
  }
}

async function saveAgentMarkdown() {
  if (saving.value || !hasChanges.value || !hasSelectedProfile.value) {
    return;
  }

  saving.value = true;
  try {
    await invoke("save_agent_profile_markdown", {
      profileId: selectedProfileId.value,
      content: content.value,
    });
    originalContent.value = content.value;
    await loadAgentProfiles();

    emitToast({
      variant: "success",
      source: "agent-config",
      message: "智能体配置已保存。",
    });
  } catch (err) {
    console.error("Failed to save agent profile markdown:", err);
  } finally {
    saving.value = false;
  }
}

async function deleteSelectedProfile() {
  if (deleting.value || !hasSelectedProfile.value) {
    return;
  }

  deleting.value = true;
  try {
    const targetName = selectedProfileLabel.value;
    const targetId = selectedProfileId.value;

    await invoke("delete_agent_profile", {
      profileId: targetId,
    });

    showDeletePanel.value = false;
    await loadAgentProfiles();

    emitToast({
      variant: "success",
      source: "agent-config",
      message: `已删除智能体: ${targetName}`,
    });
  } catch (err) {
    console.error("Failed to delete agent profile:", err);
  } finally {
    deleting.value = false;
  }
}

function resetContent() {
  content.value = originalContent.value;
}

async function handleSelectProfile(profileId: string) {
  if (!profileId || profileId === selectedProfileId.value) {
    return;
  }

  if (hasChanges.value) {
    emitToast({
      variant: "error",
      source: "agent-config",
      message: "当前有未保存内容，请先保存或撤销改动后再切换。",
    });
    return;
  }

  selectedProfileId.value = profileId;
  await loadSelectedProfileContent();
}

onMounted(() => {
  void loadAgentProfiles();
});
</script>

<template>
  <div :class="pageClass">
    <header class="flex flex-wrap items-start justify-between gap-3">
      <div class="space-y-1">
        <h2 class="text-base font-semibold text-[#111827] dark:text-[#f3f4f6]">智能体配置</h2>
        <p class="text-sm text-[#64748b] dark:text-[#a3a3a3]">
          智能体列表保存在应用数据目录，支持按条目编辑 agent markdown。
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          :class="primaryButtonClass"
          :disabled="isBusy"
          @click="openCreatePanel"
        >
          添加智能体
        </Button>
        <Button
          variant="outline"
          size="sm"
          class="h-8 border-[#fecaca] bg-white px-3 text-[13px] text-[#dc2626] shadow-none hover:bg-[#fef2f2] dark:border-[#513030] dark:bg-[#242424] dark:text-[#fca5a5] dark:hover:bg-[#3a1f1f]"
          :disabled="isBusy || !hasSelectedProfile"
          @click="openDeletePanel"
        >
          删除智能体
        </Button>
        <Button
          variant="outline"
          size="sm"
          :class="headerButtonClass"
          @click="emit('change-main-view', 'chat')"
        >
          返回聊天
        </Button>
        <Button
          variant="outline"
          size="sm"
          :class="headerButtonClass"
          :disabled="isBusy"
          @click="loadAgentProfiles"
        >
          刷新
        </Button>
        <Button
          variant="outline"
          size="sm"
          :class="headerButtonClass"
          :disabled="loadingContent || saving || !hasChanges"
          @click="resetContent"
        >
          撤销改动
        </Button>
        <Button
          size="sm"
          :class="primaryButtonClass"
          :disabled="loadingContent || saving || !hasChanges || !hasSelectedProfile"
          @click="saveAgentMarkdown"
        >
          {{ saving ? "保存中..." : "保存" }}
        </Button>
      </div>
    </header>

    <Card
      v-if="showCreatePanel"
      :class="panelClass"
    >
      <CardHeader class="space-y-1 px-3 pb-0">
        <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">创建智能体</CardTitle>
        <CardDescription>输入智能体名称，文件将保存到应用数据目录。</CardDescription>
      </CardHeader>
      <CardContent class="space-y-3 px-3">
        <Input
          v-model="newProfileName"
          :class="fieldClass"
          placeholder="例如: code-review-agent"
          :disabled="creating"
          @keydown.enter.prevent="createAgentProfile"
        />
        <div class="flex items-center justify-end gap-2">
          <Button variant="outline" size="sm" :class="headerButtonClass" :disabled="creating" @click="cancelCreatePanel">
            取消
          </Button>
          <Button
            size="sm"
            :class="primaryButtonClass"
            :disabled="creating"
            @click="createAgentProfile"
          >
            {{ creating ? "创建中..." : "确认创建" }}
          </Button>
        </div>
      </CardContent>
    </Card>

    <Card
      v-if="showDeletePanel"
      class="gap-3 border-[#fecaca] bg-[#fffafa] py-3 shadow-none dark:border-[#513030] dark:bg-[#2a2020]"
    >
      <CardHeader class="space-y-1 px-3 pb-0">
        <CardTitle class="text-sm text-[#b91c1c] dark:text-[#fca5a5]">确认删除智能体</CardTitle>
        <CardDescription>
          将删除 {{ selectedProfileLabel }} 对应的 markdown 文件，此操作不可恢复。
        </CardDescription>
      </CardHeader>
      <CardContent class="space-y-3 px-3">
        <div class="rounded-md border border-[#fecaca] bg-white px-3 py-2 text-xs text-[#b91c1c] dark:border-[#513030] dark:bg-[#241c1c] dark:text-[#fca5a5]">
          {{ selectedProfilePath || "未找到路径" }}
        </div>
        <div class="flex items-center justify-end gap-2">
          <Button variant="outline" size="sm" :class="headerButtonClass" :disabled="deleting" @click="cancelDeletePanel">
            取消
          </Button>
          <Button
            variant="destructive"
            size="sm"
            :disabled="deleting"
            @click="deleteSelectedProfile"
          >
            {{ deleting ? "删除中..." : "确认删除" }}
          </Button>
        </div>
      </CardContent>
    </Card>

    <Card
      v-if="loadingList"
      :class="panelClass"
    >
      <CardContent class="px-3 text-sm text-[#64748b] dark:text-[#a3a3a3]">正在读取智能体列表...</CardContent>
    </Card>

    <div v-else class="grid min-h-[420px] flex-1 grid-cols-[260px_minmax(0,1fr)] gap-3">
      <Card :class="panelClass">
        <CardHeader class="space-y-1 px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">智能体列表</CardTitle>
          <CardDescription>
            共 {{ profiles.length }} 个智能体
          </CardDescription>
        </CardHeader>

        <CardContent class="px-2.5">
          <div class="max-h-[calc(100vh-280px)] space-y-1 overflow-y-auto pr-1 custom-scrollbar">
            <Button
              v-for="item in profiles"
              :key="item.id"
              variant="ghost"
              class="h-auto w-full justify-start rounded-lg border px-2.5 py-2 text-left shadow-none"
              :class="item.id === selectedProfileId
                ? 'border-[#93c5fd] bg-[#eff6ff] text-[#111827] hover:bg-[#eff6ff] dark:border-[#1d4ed8] dark:bg-[#1e293b] dark:text-[#f3f4f6] dark:hover:bg-[#1e293b]'
                : 'border-transparent text-[#475569] hover:border-[#e5e7eb] hover:bg-[#f8fafc] dark:text-[#cfcfcf] dark:hover:border-[#333] dark:hover:bg-[#2a2a2a]'"
              @click="handleSelectProfile(item.id)"
            >
              <div class="w-full">
                <div class="truncate text-[13px] font-medium">{{ item.name }}</div>
                <div class="truncate text-[11px] opacity-75">{{ formatUpdatedAt(item.updatedAt) }}</div>
              </div>
            </Button>

            <div
              v-if="!hasProfiles"
              class="rounded-lg border border-dashed border-[#d8dee8] px-3 py-4 text-xs text-[#64748b] dark:border-[#3a3a3a] dark:text-[#a3a3a3]"
            >
              暂无智能体，点击上方“添加智能体”。
            </div>
          </div>
        </CardContent>
      </Card>

      <Card :class="panelClass">
        <CardHeader class="space-y-1 px-3 pb-0">
          <CardTitle class="text-sm text-[#111827] dark:text-[#f3f4f6]">{{ selectedProfileLabel }}</CardTitle>
          <CardDescription v-if="selectedProfilePath" class="break-all">
            {{ selectedProfilePath }}
          </CardDescription>
          <CardDescription v-if="hasChanges" class="text-[#2563eb] dark:text-[#93c5fd]">
            当前有未保存改动
          </CardDescription>
        </CardHeader>

        <CardContent class="h-full px-3">
          <div
            v-if="loadingContent"
            class="flex h-full min-h-[440px] items-center justify-center text-sm text-[#64748b] dark:text-[#a3a3a3]"
          >
            正在读取智能体配置...
          </div>

          <div
            v-else-if="!hasSelectedProfile"
            class="flex h-full min-h-[440px] items-center justify-center text-sm text-[#64748b] dark:text-[#a3a3a3]"
          >
            请选择或创建一个智能体。
          </div>

          <Textarea
            v-else
            v-model="content"
            class="min-h-[440px] w-full resize-y border-[#d8dee8] bg-white font-mono text-[13px] leading-6 text-[#111827] shadow-none focus-visible:border-[#2563eb] focus-visible:ring-[#2563eb]/15 dark:border-[#3a3a3a] dark:bg-[#202020] dark:text-[#ededed] dark:focus-visible:border-[#60a5fa]"
            spellcheck="false"
            placeholder="# Agent\n\n在这里编写智能体配置..."
          />
        </CardContent>
      </Card>
    </div>
  </div>
</template>
