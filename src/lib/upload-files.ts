import type {
  PendingUploadFile,
  UploadedDocumentFile,
  UploadedImageFile,
} from "./chat-types";
import { extensionOf, parseDocumentUploadFile } from "./document-upload";
import { emitToast } from "./toast";

export const MAX_UPLOAD_FILE_SIZE_BYTES = 100 * 1024 * 1024;
// 图片最终 base64 编码后的上限（约 5MB），超出会自动缩放
export const MAX_IMAGE_BASE64_BYTES = 5 * 1024 * 1024;
const IMAGE_MAX_DIMENSION = 2000;
const IMAGE_RESIZE_SCALE_STEPS = 0.75;
const IMAGE_JPEG_QUALITIES = [0.85, 0.7, 0.55, 0.4];
const SUPPORTED_IMAGE_MIME_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
]);
const IMAGE_EXTENSION_TO_MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  webp: "image/webp",
  gif: "image/gif",
};
const IMAGE_MIME_TO_EXTENSION: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
  "image/gif": "gif",
};

export function inferImageMimeType(file: File): string | null {
  const normalizedMime = (file.type || "").trim().toLowerCase();
  if (normalizedMime && SUPPORTED_IMAGE_MIME_TYPES.has(normalizedMime)) {
    return normalizedMime;
  }
  const ext = extensionOf(file.name);
  return IMAGE_EXTENSION_TO_MIME[ext] || null;
}

const readAsDataUrl = (file: File): Promise<string> =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        resolve(reader.result);
        return;
      }
      reject(new Error("无法读取文件数据"));
    };
    reader.onerror = () => {
      reject(reader.error ?? new Error("读取文件失败"));
    };
    reader.readAsDataURL(file);
  });

const loadImageElement = (dataUrl: string): Promise<HTMLImageElement> =>
  new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("图片解码失败"));
    img.src = dataUrl;
  });

const canvasToDataUrl = (
  img: HTMLImageElement,
  width: number,
  height: number,
  mime: string,
  quality?: number,
): string => {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas 2D 上下文不可用");
  ctx.drawImage(img, 0, 0, width, height);
  return quality !== undefined ? canvas.toDataURL(mime, quality) : canvas.toDataURL(mime);
};

const base64ByteLength = (dataUrl: string): number => {
  const commaIndex = dataUrl.indexOf(",");
  if (commaIndex < 0) return 0;
  const base64 = dataUrl.slice(commaIndex + 1);
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  return Math.floor((base64.length * 3) / 4) - padding;
};

// 缩放图片到符合 MAX_IMAGE_BASE64_BYTES 限制；保持原 mime（gif 不缩放，直接返回原 dataUrl）
const resizeImageIfNeeded = async (
  dataUrl: string,
  mimeType: string,
): Promise<{ dataUrl: string; mimeType: string }> => {
  if (mimeType === "image/gif") {
    return { dataUrl, mimeType };
  }

  const originalBytes = base64ByteLength(dataUrl);
  if (originalBytes <= MAX_IMAGE_BASE64_BYTES) {
    return { dataUrl, mimeType };
  }

  const img = await loadImageElement(dataUrl);
  const originalWidth = img.naturalWidth;
  const originalHeight = img.naturalHeight;

  // 计算初始缩放比例：先限制最大尺寸，再逐级 ×0.75 降采样
  const initialScale = Math.min(
    1,
    IMAGE_MAX_DIMENSION / originalWidth,
    IMAGE_MAX_DIMENSION / originalHeight,
  );
  let currentWidth = Math.max(1, Math.round(originalWidth * initialScale));
  let currentHeight = Math.max(1, Math.round(originalHeight * initialScale));

  // 尝试当前尺寸 + 逐级降采样，每级尝试 PNG（无损）+ 多档 JPEG
  while (currentWidth >= 1 && currentHeight >= 1) {
    const encoders: Array<{ mime: string; quality?: number }> = [
      { mime: mimeType },
      ...IMAGE_JPEG_QUALITIES.map((q) => ({ mime: "image/jpeg", quality: q })),
    ];
    for (const encoder of encoders) {
      const candidate = canvasToDataUrl(img, currentWidth, currentHeight, encoder.mime, encoder.quality);
      if (base64ByteLength(candidate) <= MAX_IMAGE_BASE64_BYTES) {
        return { dataUrl: candidate, mimeType: encoder.mime };
      }
    }
    if (currentWidth === 1 && currentHeight === 1) break;
    currentWidth = Math.max(1, Math.floor(currentWidth * IMAGE_RESIZE_SCALE_STEPS));
    currentHeight = Math.max(1, Math.floor(currentHeight * IMAGE_RESIZE_SCALE_STEPS));
  }

  throw new Error(`图片缩放后仍超过 ${Math.round(MAX_IMAGE_BASE64_BYTES / 1024 / 1024)}MB 限制`);
};

const fallbackPastedImageName = (mimeType: string, index: number) => {
  const ext = IMAGE_MIME_TO_EXTENSION[mimeType] || "png";
  return `pasted-image-${Date.now()}-${index + 1}.${ext}`;
};

/**
 * 把浏览器 File（文件选择器 / 粘贴 / 拖拽）解析为待上传项。
 * 返回 accepted（可上传）与 rejected（原因说明，用于提示用户）。
 */
export async function buildPendingUploadFiles(files: File[]): Promise<{
  accepted: PendingUploadFile[];
  rejected: string[];
}> {
  const accepted: PendingUploadFile[] = [];
  const rejected: string[] = [];

  for (let i = 0; i < files.length; i += 1) {
    const file = files[i];
    const imageMimeType = inferImageMimeType(file);
    if (imageMimeType) {
      if (file.size > MAX_UPLOAD_FILE_SIZE_BYTES) {
        rejected.push(`${file.name || `图片${i + 1}`}: 超过 100MB 限制`);
        continue;
      }

      let dataUrl: string;
      try {
        dataUrl = await readAsDataUrl(file);
      } catch {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片读取失败`);
        continue;
      }

      let finalMimeType = imageMimeType;
      try {
        const result = await resizeImageIfNeeded(dataUrl, imageMimeType);
        dataUrl = result.dataUrl;
        finalMimeType = result.mimeType;
      } catch (error) {
        const message = error instanceof Error ? error.message : "图片缩放失败";
        rejected.push(`${file.name || `图片${i + 1}`}: ${message}`);
        continue;
      }

      const commaIndex = dataUrl.indexOf(",");
      if (commaIndex < 0) {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片数据格式无效`);
        continue;
      }

      const base64Data = dataUrl.slice(commaIndex + 1).trim();
      if (!base64Data) {
        rejected.push(`${file.name || `图片${i + 1}`}: 图片数据为空`);
        continue;
      }

      const imageItem: UploadedImageFile = {
        kind: "image",
        sourceName: file.name || fallbackPastedImageName(finalMimeType, i),
        mimeType: finalMimeType,
        mediaType: finalMimeType,
        data: base64Data,
        size: base64ByteLength(dataUrl),
      };
      accepted.push(imageItem);
      continue;
    }

    if (file.size > MAX_UPLOAD_FILE_SIZE_BYTES) {
      rejected.push(`${file.name || `文件${i + 1}`}: 超过 100MB 限制`);
      continue;
    }

    const ext = extensionOf(file.name);
    const isBinaryDoc = ext === "docx" || ext === "pptx" || ext === "pdf";

    if (isBinaryDoc) {
      let rawBytes: number[];
      try {
        const buf = await file.arrayBuffer();
        rawBytes = Array.from(new Uint8Array(buf));
      } catch {
        rejected.push(`${file.name || `文件${i + 1}`}: 文件读取失败`);
        continue;
      }

      let content: string | null = null;
      if (ext === "docx" || ext === "pptx") {
        try {
          const parsed = await parseDocumentUploadFile(file);
          content = parsed.content;
        } catch (error) {
          const message = error instanceof Error ? error.message : "文件解析失败";
          rejected.push(`${file.name || `文件${i + 1}`}: ${message}`);
          continue;
        }
      }

      const textItem: UploadedDocumentFile = {
        kind: "document",
        sourceName: file.name,
        mimeType: file.type || undefined,
        content,
        rawBytes,
        size: file.size,
      };
      accepted.push(textItem);
      continue;
    }

    // 纯文本类文件：直接读取内容，注入对话上下文
    let textContent: string;
    try {
      textContent = await file.text();
    } catch {
      rejected.push(`${file.name || `文件${i + 1}`}: 文件读取失败`);
      continue;
    }

    const textItem: UploadedDocumentFile = {
      kind: "document",
      sourceName: file.name,
      mimeType: file.type || undefined,
      content: textContent,
      rawBytes: null,
      size: file.size,
    };
    accepted.push(textItem);
  }

  return { accepted, rejected };
}

export function notifyRejectedUploads(rejected: string[]): void {
  if (rejected.length <= 0) {
    return;
  }

  emitToast({
    variant: "error",
    source: "upload",
    message: `以下文件未导入：${rejected.slice(0, 2).join("；")}`,
  });
}
