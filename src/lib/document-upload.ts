import type JSZip from "jszip";
import type { TextItem } from "pdfjs-dist/types/src/display/api";

async function getJSZip() {
  const mod = await import("jszip");
  return mod.default;
}

async function getPdfJs() {
  const pdfjsLib = await import("pdfjs-dist");
  if (!pdfjsLib.GlobalWorkerOptions.workerSrc) {
    pdfjsLib.GlobalWorkerOptions.workerSrc = new URL(
      "pdfjs-dist/build/pdf.worker.min.mjs",
      import.meta.url,
    ).toString();
  }
  return pdfjsLib;
}

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

export const LEGACY_OFFICE_EXTENSIONS = ["doc", "ppt"] as const;

export const SUPPORTED_DOCUMENT_EXTENSION_SET = new Set<string>(SUPPORTED_DOCUMENT_EXTENSIONS);
const LEGACY_OFFICE_EXTENSION_SET = new Set<string>(LEGACY_OFFICE_EXTENSIONS);

type ParsedDocumentKind = "plain_text" | "docx" | "pptx" | "pdf";

export type ParsedDocumentUpload = {
  content: string;
  kind: ParsedDocumentKind;
  extension: string;
};

const WORD_OPTIONAL_PART_LABELS: Array<[pattern: RegExp, label: string]> = [
  [/^word\/footnotes\.xml$/i, "Footnotes"],
  [/^word\/endnotes\.xml$/i, "Endnotes"],
  [/^word\/comments\.xml$/i, "Comments"],
  [/^word\/header\d+\.xml$/i, "Header"],
  [/^word\/footer\d+\.xml$/i, "Footer"],
];

function collapseConsecutiveBlankLines(input: string): string {
  return input
    .replace(/\r\n/g, "\n")
    .replace(/\u000b/g, "\n")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function joinNonEmpty(parts: string[], separator: string): string {
  return parts.filter((part) => part.trim().length > 0).join(separator);
}

function parseXml(xml: string): XMLDocument {
  const doc = new DOMParser().parseFromString(xml, "application/xml");
  if (doc.getElementsByTagName("parsererror").length > 0) {
    throw new Error("文档 XML 解析失败");
  }
  return doc;
}

function childElements(node: Node, localName?: string): Element[] {
  return Array.from(node.childNodes).filter((child): child is Element => {
    return child.nodeType === Node.ELEMENT_NODE
      && (!localName || (child as Element).localName === localName);
  });
}

function extractWordNodeText(node: Node): string {
  if (node.nodeType !== Node.ELEMENT_NODE) {
    return node.textContent ?? "";
  }

  const element = node as Element;
  switch (element.localName) {
    case "t":
      return element.textContent ?? "";
    case "tab":
      return "\t";
    case "br":
    case "cr":
      return "\n";
    case "tbl": {
      const rows = childElements(element, "tr")
        .map((row) => collapseConsecutiveBlankLines(extractWordNodeText(row)))
        .filter(Boolean);
      return rows.length > 0 ? `${rows.join("\n")}\n\n` : "";
    }
    case "tr": {
      const cells = childElements(element, "tc")
        .map((cell) => collapseConsecutiveBlankLines(extractWordNodeText(cell)))
        .filter(Boolean);
      return cells.join("\t");
    }
    case "p": {
      const text = Array.from(element.childNodes).map(extractWordNodeText).join("");
      const normalized = collapseConsecutiveBlankLines(text);
      return normalized ? `${normalized}\n\n` : "";
    }
    default:
      return Array.from(element.childNodes).map(extractWordNodeText).join("");
  }
}

function extractWordXmlText(xml: string, rootLocalName?: string): string {
  const doc = parseXml(xml);
  const root = rootLocalName
    ? doc.getElementsByTagNameNS("*", rootLocalName).item(0) ?? doc.documentElement
    : doc.documentElement;
  return collapseConsecutiveBlankLines(extractWordNodeText(root));
}

function extractPresentationNodeText(node: Node): string {
  if (node.nodeType !== Node.ELEMENT_NODE) {
    return node.textContent ?? "";
  }

  const element = node as Element;
  switch (element.localName) {
    case "t":
      return element.textContent ?? "";
    case "tab":
      return "\t";
    case "br":
      return "\n";
    case "p": {
      const text = Array.from(element.childNodes).map(extractPresentationNodeText).join("");
      const normalized = collapseConsecutiveBlankLines(text);
      return normalized ? `${normalized}\n` : "";
    }
    default:
      return Array.from(element.childNodes).map(extractPresentationNodeText).join("");
  }
}

function extractPresentationXmlText(xml: string): string {
  const doc = parseXml(xml);
  return collapseConsecutiveBlankLines(extractPresentationNodeText(doc.documentElement));
}

function numericSuffix(path: string): number {
  const match = path.match(/(\d+)\.xml$/i);
  return match ? Number.parseInt(match[1], 10) : Number.MAX_SAFE_INTEGER;
}

async function readZipText(zip: JSZip, path: string): Promise<string> {
  const entry = zip.file(path);
  if (!entry) {
    throw new Error(`缺少文件 ${path}`);
  }
  return entry.async("text");
}

async function parseDocxFile(file: File): Promise<ParsedDocumentUpload> {
  const JSZip = await getJSZip();
  const zip = await JSZip.loadAsync(await file.arrayBuffer());
  const sections: string[] = [];

  const mainDocument = extractWordXmlText(await readZipText(zip, "word/document.xml"), "body");
  if (mainDocument) {
    sections.push(mainDocument);
  }

  for (const [pattern, label] of WORD_OPTIONAL_PART_LABELS) {
    const paths = Object.keys(zip.files)
      .filter((path) => pattern.test(path))
      .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));

    for (const path of paths) {
      const text = extractWordXmlText(await readZipText(zip, path));
      if (text) {
        sections.push(`${label}\n${text}`);
      }
    }
  }

  const content = collapseConsecutiveBlankLines(joinNonEmpty(sections, "\n\n"));
  if (!content) {
    throw new Error("Word 文档中未提取到可读文本");
  }

  return {
    content,
    kind: "docx",
    extension: "docx",
  };
}

async function parsePptxFile(file: File): Promise<ParsedDocumentUpload> {
  const JSZip = await getJSZip();
  const zip = await JSZip.loadAsync(await file.arrayBuffer());
  const slidePaths = Object.keys(zip.files)
    .filter((path) => /^ppt\/slides\/slide\d+\.xml$/i.test(path))
    .sort((a, b) => numericSuffix(a) - numericSuffix(b));

  if (slidePaths.length === 0) {
    throw new Error("PPTX 文件中未找到幻灯片内容");
  }

  const slides: string[] = [];
  for (const [index, slidePath] of slidePaths.entries()) {
    const slideText = extractPresentationXmlText(await readZipText(zip, slidePath));
    const notesPath = `ppt/notesSlides/notesSlide${numericSuffix(slidePath)}.xml`;
    const notesEntry = zip.file(notesPath);
    const notesText = notesEntry
      ? extractPresentationXmlText(await notesEntry.async("text"))
      : "";

    const parts = [`Slide ${index + 1}`];
    if (slideText) {
      parts.push(slideText);
    }
    if (notesText) {
      parts.push(`Notes\n${notesText}`);
    }

    const slideSection = collapseConsecutiveBlankLines(joinNonEmpty(parts, "\n"));
    if (slideSection) {
      slides.push(slideSection);
    }
  }

  const content = collapseConsecutiveBlankLines(joinNonEmpty(slides, "\n\n"));
  if (!content) {
    throw new Error("PPT 文档中未提取到可读文本");
  }

  return {
    content,
    kind: "pptx",
    extension: "pptx",
  };
}

function unsupportedLegacyOfficeMessage(extension: string): string {
  return `暂不支持旧版 Office 格式 .${extension}，请先另存为 .${extension}x 后再上传`;
}

export function extensionOf(fileName: string): string {
  const idx = fileName.lastIndexOf(".");
  if (idx < 0) return "";
  return fileName.slice(idx + 1).toLowerCase();
}

export function isSupportedDocumentExtension(extension: string): boolean {
  return SUPPORTED_DOCUMENT_EXTENSION_SET.has(extension);
}

export function isLegacyOfficeExtension(extension: string): boolean {
  return LEGACY_OFFICE_EXTENSION_SET.has(extension);
}

export function describeSupportedDocumentExtensions(): string {
  return SUPPORTED_DOCUMENT_EXTENSIONS.join(", ");
}

export function buildDocumentAcceptAttribute(includeImages = false): string {
  const documentPatterns = SUPPORTED_DOCUMENT_EXTENSIONS.map((ext) => `.${ext}`);
  if (!includeImages) {
    return documentPatterns.join(",");
  }
  return [...documentPatterns, "image/png", "image/jpeg", "image/webp", "image/gif"].join(",");
}

async function parsePdfFile(file: File): Promise<ParsedDocumentUpload> {
  const pdfjsLib = await getPdfJs();
  const arrayBuffer = await file.arrayBuffer();
  const pdf = await pdfjsLib.getDocument({ data: arrayBuffer }).promise;
  const pages: string[] = [];
  for (let i = 1; i <= pdf.numPages; i++) {
    const page = await pdf.getPage(i);
    const textContent = await page.getTextContent();
    const pageText = textContent.items
      .filter((item): item is TextItem => "str" in item)
      .map((item) => item.str)
      .join(" ");
    const normalized = collapseConsecutiveBlankLines(pageText);
    if (normalized) pages.push(`Page ${i}\n${normalized}`);
  }
  const content = collapseConsecutiveBlankLines(pages.join("\n\n"));
  if (!content) throw new Error("PDF 中未提取到可读文本");
  return { content, kind: "pdf", extension: "pdf" };
}

export async function parseDocumentUploadFile(file: File): Promise<ParsedDocumentUpload> {
  const extension = extensionOf(file.name);
  if (isLegacyOfficeExtension(extension)) {
    throw new Error(unsupportedLegacyOfficeMessage(extension));
  }
  if (!isSupportedDocumentExtension(extension)) {
    throw new Error("不支持的文件类型");
  }

  if (extension === "docx") {
    return parseDocxFile(file);
  }
  if (extension === "pptx") {
    return parsePptxFile(file);
  }
  if (extension === "pdf") {
    return parsePdfFile(file);
  }

  const content = collapseConsecutiveBlankLines(await file.text());
  if (!content) {
    throw new Error("文件内容为空");
  }

  return {
    content,
    kind: "plain_text",
    extension,
  };
}
