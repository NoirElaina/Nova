<script setup lang="ts">
import { useRouter } from "vue-router";
import ScheduleTaskScreen from "@/components/schedule/ScheduleTaskScreen.vue";
import { useChatController } from "@/features/chat/controllers/useChatController";

const router = useRouter();
const { handleSelectConversation } = useChatController();

async function handleChangeMainView(view: "chat" | "hooks" | "agent" | "schedule") {
  if (view === "chat") {
    await router.push("/chat");
  } else {
    await router.push(`/${view}`);
  }
}

async function handleOpenTaskConversation(conversationId: string) {
  await handleSelectConversation(conversationId);
  await router.push(`/chat/${conversationId}`);
}
</script>

<template>
  <div class="h-full w-full overflow-hidden">
    <ScheduleTaskScreen
      @change-main-view="handleChangeMainView"
      @open-task-conversation="handleOpenTaskConversation"
    />
  </div>
</template>
