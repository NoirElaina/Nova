import { ref, type Ref, type ComputedRef } from 'vue';
import type { PendingUploadFile } from '@/lib/chat-types';
import { buildDocumentAcceptAttribute } from '@/lib/document-upload';
import {
  buildPendingUploadFiles,
  inferImageMimeType,
  notifyRejectedUploads,
} from '@/lib/upload-files';
import { useComposerStore } from '@/stores/composer';

export interface UseFileInputOptions {
  isGenerating: Ref<boolean> | ComputedRef<boolean>;
  onUploadFiles?: (files: PendingUploadFile[]) => void;
  onRemoveUpload?: (index: number) => void;
}

export function useFileInput(options: UseFileInputOptions) {
  const { isGenerating, onUploadFiles, onRemoveUpload } = options;
  const composerStore = useComposerStore();
  const fileInputRef = ref<HTMLInputElement | null>(null);

  const FILE_INPUT_ACCEPT = buildDocumentAcceptAttribute(true);

  const triggerFilePicker = () => {
    if (isGenerating.value) return;
    fileInputRef.value?.click();
  };

  const onFileChange = async (event: Event) => {
    const input = event.target as HTMLInputElement;
    const files = input.files ? Array.from(input.files) : [];
    if (files.length === 0) {
      return;
    }

    const { accepted, rejected } = await buildPendingUploadFiles(files);

    if (accepted.length > 0) {
      composerStore.addUploads(accepted);
      onUploadFiles?.(accepted);
    }

    notifyRejectedUploads(rejected);
    input.value = '';
  };

  const onTextareaPaste = async (event: ClipboardEvent) => {
    if (isGenerating.value) return;

    const clipboardData = event.clipboardData;
    if (!clipboardData) {
      return;
    }

    const itemFiles = Array.from(clipboardData.items ?? [])
      .filter((item) => item.kind === 'file')
      .map((item) => item.getAsFile())
      .filter((file): file is File => !!file);
    const files = itemFiles.length > 0 ? itemFiles : Array.from(clipboardData.files ?? []);
    if (files.length === 0) {
      return;
    }

    const imageFiles = files.filter((file) => !!inferImageMimeType(file));
    if (imageFiles.length === 0) {
      return;
    }

    event.preventDefault();
    const { accepted, rejected } = await buildPendingUploadFiles(imageFiles);
    if (accepted.length > 0) {
      composerStore.addUploads(accepted);
      onUploadFiles?.(accepted);
    }
    notifyRejectedUploads(rejected);
  };

  const handleRemoveUpload = (index: number) => {
    composerStore.removeUpload(index);
    onRemoveUpload?.(index);
  };

  return {
    fileInputRef,
    FILE_INPUT_ACCEPT,
    triggerFilePicker,
    onFileChange,
    onTextareaPaste,
    handleRemoveUpload,
  };
}
