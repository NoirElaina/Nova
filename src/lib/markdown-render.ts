import MarkdownIt from "markdown-it";
import hljs from "highlight.js";
import markdownItKatex from "@traptitech/markdown-it-katex";

const COPY_ICON_SVG =
  '<svg class="hljs-copy-icon" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
const CHECK_ICON_SVG =
  '<svg class="hljs-copy-icon is-check" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';

function highlightCode(code: string, lang: string): { lang: string; html: string } {
  const normalized = (lang || "").trim().toLowerCase();
  if (normalized && hljs.getLanguage(normalized)) {
    try {
      return {
        lang: normalized,
        html: hljs.highlight(code, { language: normalized }).value,
      };
    } catch {
      // fall through
    }
  }
  return {
    lang: normalized || "text",
    html: md.utils.escapeHtml(code),
  };
}

function buildCodeBlockHtml(lang: string, highlightedInner: string): string {
  const label = (lang || "text").trim() || "text";
  const safeLang = md.utils.escapeHtml(label);
  // 注意：不能再被 markdown-it 包一层 <pre><code>，否则会出现空白碎片块
  return `<div class="hljs-block"><div class="hljs-header"><span class="hljs-lang">${safeLang}</span><button type="button" class="hljs-copy" title="复制" aria-label="复制">${COPY_ICON_SVG}${CHECK_ICON_SVG}</button></div><pre class="hljs-pre"><code class="hljs language-${safeLang}">${highlightedInner}</code></pre></div>`;
}

const md = new MarkdownIt({
  html: true,
  linkify: true,
  typographer: true,
});

md.use(markdownItKatex);

// 自定义 fence：一次输出完整代码块 DOM（语言标签 + 复制 + 高亮）
// 不用 options.highlight，避免外层再包 <pre><code> 留下空白碎片
md.renderer.rules.fence = (tokens, idx) => {
  const token = tokens[idx];
  const info = token.info ? md.utils.unescapeAll(token.info).trim() : "";
  const langName = info ? info.split(/\s+/g)[0] : "";
  const { lang, html } = highlightCode(token.content, langName);
  return `${buildCodeBlockHtml(lang, html)}\n`;
};

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const defaultLinkOpenRender: any =
  md.renderer.rules.link_open ||
  function (tokens: any[], idx: number, options: any, _env: any, self: any) {
    return self.renderToken(tokens, idx, options);
  };

// eslint-disable-next-line @typescript-eslint/no-explicit-any
md.renderer.rules.link_open = function (
  tokens: any[],
  idx: number,
  options: any,
  env: any,
  self: any,
): string {
  tokens[idx].attrSet("target", "_blank");
  tokens[idx].attrSet("rel", "noopener noreferrer");
  return defaultLinkOpenRender(tokens, idx, options, env, self);
};

/** 闭合正文 HTML 缓存：历史气泡与已闭合 segment 复用，避免重复 parse。 */
const RENDER_CACHE_LIMIT = 200;
/** 结构变更时递增，避免旧 HTML（无 sticky 头/旧主题）被缓存命中 */
const RENDER_CACHE_VERSION = "codeblock-v5-light-fence";
const renderCache = new Map<string, string>();

function cacheKey(source: string): string {
  return `${RENDER_CACHE_VERSION}\n${source}`;
}

function cacheGet(source: string): string | undefined {
  const key = cacheKey(source);
  const hit = renderCache.get(key);
  if (hit === undefined) return undefined;
  // 简单 LRU：命中后挪到末尾
  renderCache.delete(key);
  renderCache.set(key, hit);
  return hit;
}

function cacheSet(source: string, value: string) {
  const key = cacheKey(source);
  if (renderCache.has(key)) {
    renderCache.delete(key);
  }
  renderCache.set(key, value);
  while (renderCache.size > RENDER_CACHE_LIMIT) {
    const oldest = renderCache.keys().next().value;
    if (oldest === undefined) break;
    renderCache.delete(oldest);
  }
}

/** 给尚未包裹的 table 套横向滚动容器，避免宽表撑破布局引发纵向重排 */
function wrapMarkdownTables(html: string): string {
  return html.replace(/<table\b[\s\S]*?<\/table>/gi, (block, offset, full) => {
    const before = full.slice(Math.max(0, offset - 48), offset);
    if (before.includes("md-table-wrap")) return block;
    return `<div class="md-table-wrap">${block}</div>`;
  });
}

const TABLE_SEP_RE = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/;

function splitTableCells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

function padCells(cells: string[], cols: number): string[] {
  if (cells.length < cols) {
    return cells.concat(Array.from({ length: cols - cells.length }, () => ""));
  }
  if (cells.length > cols) return cells.slice(0, cols);
  return cells;
}

/** 表格单元格走 inline markdown，保证 `code`、**粗体**、链接等能渲染 */
function renderTableCell(text: string): string {
  return md.renderInline((text || "").trim());
}

/**
 * 流式输出时未闭合的 GFM 表会在「段落 / 半表」间来回切换，高度暴涨暴跌。
 * 对末尾未完成的表块补齐分隔行与空单元格，尽量保持 table 结构稳定。
 */
export function stabilizeStreamingMarkdown(content: string): string {
  const source = content || "";
  if (!source.includes("|")) return source;

  const lines = source.split("\n");
  let end = lines.length - 1;
  while (end >= 0 && lines[end].trim() === "") end -= 1;
  if (end < 0) return source;

  let start = end;
  while (start >= 0) {
    const t = lines[start].trim();
    if (t === "" || !t.includes("|")) break;
    start -= 1;
  }
  start += 1;
  if (start > end) return source;

  const tableLines = lines.slice(start, end + 1).map((l) => l.trimEnd());
  if (tableLines.length === 0 || !tableLines[0].includes("|")) return source;

  let cols = splitTableCells(tableLines[0]).length;
  if (cols < 1) return source;

  if (tableLines.length === 1 || !TABLE_SEP_RE.test(tableLines[1] ?? "")) {
    tableLines.splice(
      1,
      0,
      `| ${Array.from({ length: cols }, () => "---").join(" | ")} |`,
    );
  } else {
    cols = Math.max(cols, splitTableCells(tableLines[1]).length);
  }

  for (let i = 0; i < tableLines.length; i += 1) {
    if (TABLE_SEP_RE.test(tableLines[i])) continue;
    const cells = padCells(splitTableCells(tableLines[i]), cols);
    tableLines[i] = `| ${cells.join(" | ")} |`;
  }

  const next = [...lines];
  next.splice(start, end - start + 1, ...tableLines);
  return next.join("\n");
}

/**
 * markdown-it 默认不解析 GFM 表。预转为 HTML，避免 | 表被拆成段落导致流式高度抽搐。
 * 跳过 fenced code 内部内容。
 */
function convertGfmTablesToHtml(content: string): string {
  if (!content.includes("|")) return content;

  const lines = content.split("\n");
  const out: string[] = [];
  let inFence = false;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const fence = line.trim().startsWith("```") || line.trim().startsWith("~~~");
    if (fence) {
      inFence = !inFence;
      out.push(line);
      continue;
    }
    if (inFence) {
      out.push(line);
      continue;
    }

    const header = line.trim();
    const sep = (lines[i + 1] ?? "").trim();
    const looksLikeHeader = header.includes("|") && !header.startsWith("<");
    if (!looksLikeHeader || !TABLE_SEP_RE.test(sep)) {
      out.push(line);
      continue;
    }

    let cols = splitTableCells(header).length;
    cols = Math.max(cols, splitTableCells(sep).length);
    const headerCells = padCells(splitTableCells(header), cols);

    const body: string[][] = [];
    let j = i + 2;
    while (j < lines.length) {
      const row = lines[j].trim();
      if (!row || !row.includes("|") || row.startsWith("```") || row.startsWith("~~~")) {
        break;
      }
      // 空行结束表
      if (lines[j].trim() === "") break;
      body.push(padCells(splitTableCells(lines[j]), cols));
      j += 1;
    }

    const thead = `<thead><tr>${headerCells
      .map((c) => `<th>${renderTableCell(c)}</th>`)
      .join("")}</tr></thead>`;
    const tbody =
      body.length > 0
        ? `<tbody>${body
            .map(
              (cells) =>
                `<tr>${cells.map((c) => `<td>${renderTableCell(c)}</td>`).join("")}</tr>`,
            )
            .join("")}</tbody>`
        : "<tbody></tbody>";

    out.push(`<div class="md-table-wrap"><table>${thead}${tbody}</table></div>`);
    // 吃掉已转换的行
    i = j - 1;
  }

  return out.join("\n");
}

function renderMarkdownUncached(content: string): string {
  const withTables = convertGfmTablesToHtml(content || "");
  let html = md.render(withTables);

  html = html.replace(
    /(<details[^>]*>)([\s\S]*?)(<\/details>)/g,
    (_: string, open: string, inner: string, close: string) => {
      const processed = inner.replace(
        /(<\/summary>)([\s\S]*?)$/,
        (__: string, summaryClose: string, rest: string) => {
          const trimmed = rest.trim();
          if (!trimmed) return summaryClose;
          return `${summaryClose}<div class="details-body">${md.render(convertGfmTablesToHtml(trimmed))}</div>`;
        },
      );
      return open + processed + close;
    },
  );

  return wrapMarkdownTables(html);
}

/**
 * @param content markdown 原文
 * @param options.cache 为 true 时缓存结果（历史/已闭合 segment）；流式 open segment 应传 false
 * @param options.live 流式未闭合段：稳定半截表格，减少高度抖动
 */
export function renderMarkdown(
  content: string,
  options: { cache?: boolean; live?: boolean } = {},
): string {
  const source = options.live
    ? stabilizeStreamingMarkdown(content || "")
    : content || "";
  if (!options.cache) {
    return renderMarkdownUncached(source);
  }
  const hit = cacheGet(source);
  if (hit !== undefined) {
    return hit;
  }
  const html = renderMarkdownUncached(source);
  cacheSet(source, html);
  return html;
}
