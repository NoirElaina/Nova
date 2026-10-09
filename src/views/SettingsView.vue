<script setup lang="ts">
import { useRouter } from "vue-router";
import SettingsScreen from "@/components/settings/SettingsScreen.vue";

const router = useRouter();

async function handleChangeMainView(view: "chat" | "settings") {
  if (view === "chat") {
    await router.push("/chat");
  } else {
    await router.push(`/${view}`);
  }
}
</script>

<template>
  <div class="h-full w-full overflow-hidden">
    <SettingsScreen
      @change-main-view="handleChangeMainView"
    />
  </div>
</template>
