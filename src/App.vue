<script setup lang="ts">
import { onMounted } from "vue";
import GlobalToastHost from "./components/layout/GlobalToastHost.vue";
import { useChatController } from "./features/chat/controllers/useChatController";

// 全局启动 ChatController 单例，确保后台事件流与定时器在根生命周期中持续存活
useChatController();

onMounted(() => {
  // 全局防御浏览器拖拽文件时的默认导航行为
  const preventDefaultDrag = (event: DragEvent) => {
    if (Array.from(event.dataTransfer?.types ?? []).includes("Files")) {
      event.preventDefault();
    }
  };
  window.addEventListener("dragover", preventDefaultDrag);
  window.addEventListener("drop", preventDefaultDrag);
});
</script>

<template>
  <div class="flex h-screen w-screen overflow-hidden bg-[#fcfcfc] dark:bg-[#1a1a1a] text-[#1a1a1a] dark:text-[#ececec] font-sans">
    <GlobalToastHost />
    <RouterView />
  </div>
</template>

<style>
html, body, #app {
  margin: 0;
  padding: 0;
  width: 100%;
  height: 100%;
}
</style>
