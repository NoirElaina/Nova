import { ref, type Ref, type ComputedRef } from "vue";
import type { PendingUploadFile } from "@/lib/chat-types";
import { buildPendingUploadFiles, notifyRejectedUploads } from "@/lib/upload-files";

export interface UseChatDropzoneOptions {
  mainView: Ref<string> | ComputedRef<string>;
  onFilesAccepted: (files: PendingUploadFile[]) => Promise<void> | void;
}

export function useChatDropzone(options: UseChatDropzoneOptions) {
  const { mainView, onFilesAccepted } = options;

  const isDraggingFiles = ref(false);
  let dragDepth = 0;

  const hasDraggedFiles = (event: DragEvent) =>
    Array.from(event.dataTransfer?.types ?? []).includes("Files");

  const handleChatDragEnter = (event: DragEvent) => {
    if (mainView.value !== "chat" || !hasDraggedFiles(event)) return;
    dragDepth += 1;
    isDraggingFiles.value = true;
  };

  const handleChatDragOver = (event: DragEvent) => {
    if (!hasDraggedFiles(event)) return;
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "copy";
    }
  };

  const handleChatDragLeave = (event: DragEvent) => {
    if (!hasDraggedFiles(event)) return;
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) {
      isDraggingFiles.value = false;
    }
  };

  const handleChatDrop = async (event: DragEvent) => {
    if (mainView.value !== "chat" || !hasDraggedFiles(event)) return;
    event.preventDefault();
    dragDepth = 0;
    isDraggingFiles.value = false;

    const files = Array.from(event.dataTransfer?.files ?? []);
    if (files.length === 0) return;

    const { accepted, rejected } = await buildPendingUploadFiles(files);
    if (accepted.length > 0) {
      await onFilesAccepted(accepted);
    }
    notifyRejectedUploads(rejected);
  };

  return {
    isDraggingFiles,
    handleChatDragEnter,
    handleChatDragOver,
    handleChatDragLeave,
    handleChatDrop,
  };
}
