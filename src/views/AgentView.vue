<script setup lang="ts">
import { useRouter } from "vue-router";
import AgentConfigScreen from "@/components/agent/AgentConfigScreen.vue";
import { useChatController } from "@/features/chat/controllers/useChatController";

const router = useRouter();
const {
  activeConversationId,
  handleLaunchAgentConversation,
} = useChatController();

async function handleChangeMainView(view: "chat" | "hooks" | "agent") {
  if (view === "chat") {
    await router.push("/chat");
  } else {
    await router.push(`/${view}`);
  }
}

async function onLaunchAgent(bundleId: string) {
  await handleLaunchAgentConversation(bundleId);
  await router.push("/chat");
}
</script>

<template>
  <div class="h-full w-full overflow-hidden">
    <AgentConfigScreen
      :conversation-id="activeConversationId || null"
      @change-main-view="handleChangeMainView"
      @launch-agent="onLaunchAgent"
    />
  </div>
</template>
