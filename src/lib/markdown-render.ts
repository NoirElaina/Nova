import MarkdownIt from "markdown-it";
import hljs from "highlight.js";
import markdownItKatex from "@traptitech/markdown-it-katex";

const md = new MarkdownIt({
  html: true,
  linkify: true,
  typographer: true,
  highlight(code: string, lang: string): string {
    if (lang && hljs.getLanguage(lang)) {
      try {
        return `<pre class="hljs-block"><div class="hljs-header"><span class="hljs-lang">${lang}</span><button class="hljs-copy" onclick="navigator.clipboard.writeText(this.closest('pre').querySelector('code').innerText)">复制</button></div><code class="hljs language-${lang}">${hljs.highlight(code, { language: lang }).value}</code></pre>`;
      } catch {
        // Fall through to escaped plain code below.
      }
    }
    return `<pre class="hljs-block"><code class="hljs">${md.utils.escapeHtml(code)}</code></pre>`;
  },
});

md.use(markdownItKatex);

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

export function renderMarkdown(content: string): string {
  let html = md.render(content || "");

  html = html.replace(
    /(<details[^>]*>)([\s\S]*?)(<\/details>)/g,
    (_: string, open: string, inner: string, close: string) => {
      const processed = inner.replace(
        /(<\/summary>)([\s\S]*?)$/,
        (__: string, summaryClose: string, rest: string) => {
          const trimmed = rest.trim();
          if (!trimmed) return summaryClose;
          return `${summaryClose}<div class="details-body">${md.render(trimmed)}</div>`;
        },
      );
      return open + processed + close;
    },
  );

  return html;
}
