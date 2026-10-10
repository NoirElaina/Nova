export const SUPPORTED_PLAIN_TEXT_EXTENSIONS = [
  "txt",
  "md",
  "markdown",
  "json",
  "yaml",
  "yml",
  "toml",
  "ini",
  "log",
  "csv",
  "ts",
  "tsx",
  "js",
  "jsx",
  "py",
  "rs",
  "go",
  "java",
  "c",
  "cc",
  "cpp",
  "h",
  "hpp",
  "vue",
  "css",
  "scss",
  "html",
  "xml",
  "sql",
  "sh",
  "ps1",
  "bat",
] as const;

export const SUPPORTED_OFFICE_EXTENSIONS = ["docx", "pptx", "pdf"] as const;

export const SUPPORTED_DOCUMENT_EXTENSIONS = [
  ...SUPPORTED_PLAIN_TEXT_EXTENSIONS,
  ...SUPPORTED_OFFICE_EXTENSIONS,
] as const;

export const SUPPORTED_DOCUMENT_EXTENSION_SET = new Set<string>(SUPPORTED_DOCUMENT_EXTENSIONS);

type ParsedDocumentKind = "plain_text" | "docx" | "pptx" | "pdf";

export type ParsedDocumentUpload = {
  content: string;
  kind: ParsedDocumentKind;
  extension: string;
};

export function extensionOf(filename: string): string {
  const normalized = filename.trim().toLowerCase();
  const lastDot = normalized.lastIndexOf(".");
  if (lastDot < 0 || lastDot === normalized.length - 1) {
    return "";
  }
  return normalized.slice(lastDot + 1);
}

export function buildDocumentAcceptAttribute(includeImages: boolean = false): string {
  const exts = [...SUPPORTED_DOCUMENT_EXTENSIONS];
  if (includeImages) {
    return exts.map((ext) => `.${ext}`).join(",") + ",image/*";
  }
  return exts.map((ext) => `.${ext}`).join(",");
}

export function describeSupportedDocumentExtensions(): string {
  return SUPPORTED_DOCUMENT_EXTENSIONS.map((ext) => `.${ext}`).join("、");
}

/**
 * 轻量文件解析：纯文本直接读取内容，文档交由后端统筹处理。
 * 不再在前端嵌入庞大的 jszip 与 pdfjs-dist 库。
 */
export async function parseDocumentUploadFile(file: File): Promise<ParsedDocumentUpload> {
  const ext = extensionOf(file.name);
  if (!SUPPORTED_DOCUMENT_EXTENSION_SET.has(ext)) {
    throw new Error(`不支持的文件格式: .${ext || "unknown"}`);
  }

  // 纯文本类直接异步读取
  try {
    const text = await file.text();
    return {
      content: text,
      kind: "plain_text",
      extension: ext,
    };
  } catch (error) {
    throw new Error(`文件读取失败: ${error instanceof Error ? error.message : String(error)}`);
  }
}
