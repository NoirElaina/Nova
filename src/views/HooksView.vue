<script setup lang="ts">
import { useRouter } from "vue-router";
import HooksConfigScreen from "@/components/hooks/HooksConfigScreen.vue";

const router = useRouter();

async function handleChangeMainView(view: "chat" | "hooks") {
  if (view === "chat") {
    await router.push("/chat");
  } else {
    await router.push(`/${view}`);
  }
}
</script>

<template>
  <div class="h-full w-full overflow-hidden">
    <HooksConfigScreen
      @change-main-view="handleChangeMainView"
    />
  </div>
</template>
