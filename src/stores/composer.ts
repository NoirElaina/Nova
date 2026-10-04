import { defineStore } from "pinia";
import { ref } from "vue";
import type { AgentMode, PendingUploadFile } from "@/lib/chat-types";

export const useComposerStore = defineStore("composer", () => {
  const currentInput = ref<string>("");
  const pendingUploads = ref<PendingUploadFile[]>([]);
  const agentMode = ref<AgentMode>("agent");
  const isComposing = ref<boolean>(false);

  function setInput(text: string) {
    currentInput.value = text;
  }

  function appendInput(text: string) {
    currentInput.value += text;
  }

  function clearInput() {
    currentInput.value = "";
  }

  function addUploads(files: PendingUploadFile[]) {
    pendingUploads.value.push(...files);
  }

  function removeUpload(index: number) {
    if (index >= 0 && index < pendingUploads.value.length) {
      pendingUploads.value.splice(index, 1);
    }
  }

  function clearUploads() {
    pendingUploads.value = [];
  }

  return {
    currentInput,
    pendingUploads,
    agentMode,
    isComposing,
    setInput,
    appendInput,
    clearInput,
    addUploads,
    removeUpload,
    clearUploads,
  };
});
