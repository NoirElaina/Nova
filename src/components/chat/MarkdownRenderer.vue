<script setup lang="ts">
import { computed } from 'vue'
import { Card, CardContent } from '@/components/ui/card'
import { renderMarkdown } from '@/lib/markdown-render'

const props = withDefaults(
  defineProps<{
    content: string
    /** 流式未闭合段：不走缓存，保证实时；历史/已闭合段：缓存 HTML */
    live?: boolean
  }>(),
  { live: false },
)

const rendered = computed(() =>
  renderMarkdown(props.content || '', {
    cache: !props.live,
    live: props.live,
  }),
)

const COPY_ICON =
  '<svg class="hljs-copy-icon" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>'
const CHECK_ICON =
  '<svg class="hljs-copy-icon is-check" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>'

const handleClick = async (event: MouseEvent) => {
  const target = event.target
  if (!(target instanceof Element)) return
  const btn = target.closest('.hljs-copy')
  if (!(btn instanceof HTMLButtonElement)) return

  const block = btn.closest('.hljs-block')
  const code = block?.querySelector('code')
  const text = code?.textContent ?? ''
  if (!text) return

  const restore = () => {
    btn.classList.remove('is-copied')
  }

  try {
    await navigator.clipboard.writeText(text)
    btn.classList.add('is-copied')
    window.setTimeout(restore, 800)
  } catch {
    window.setTimeout(restore, 800)
  }
}
</script>

<template>
  <Card class="border-0 bg-transparent py-0 shadow-none">
    <CardContent class="px-0">
      <div class="md-body" v-html="rendered" @click="handleClick" />
    </CardContent>
  </Card>
</template>

<style>
@import 'katex/dist/katex.min.css';

.md-body {
  font-size: 14px;
  line-height: 1.75;
  color: inherit;
  word-break: break-word;
  overflow-x: auto;
  width: 100%;
  max-width: 100%;
}

.md-body>*:first-child {
  margin-top: 0 !important;
}

.md-body>*:last-child {
  margin-bottom: 0 !important;
}

.md-body h1,
.md-body h2,
.md-body h3,
.md-body h4,
.md-body h5,
.md-body h6 {
  font-weight: 700;
  line-height: 1.3;
  margin: 1.2em 0 0.5em;
  color: #111827;
}

.md-body h1 {
  font-size: 1.5em;
}

.md-body h2 {
  font-size: 1.25em;
  border-bottom: 1px solid #e5e7eb;
  padding-bottom: 0.3em;
}

.md-body h3 {
  font-size: 1.1em;
}

.md-body p {
  margin: 0.6em 0;
}

.md-body strong {
  font-weight: 700;
  color: #111827;
}

.md-body em {
  font-style: italic;
}

.md-body code:not(.hljs) {
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  font-size: 0.85em;
  background: #f6f1e9;
  color: #2f3a4a;
  padding: 1px 5px;
  border-radius: 4px;
  border: 1px solid #e4ddd3;
  box-shadow: inset 0 -1px 0 rgba(120, 92, 64, 0.08);
}

/* 浅色代码块：GitHub-light 风格，头部与正文有区分 */
.hljs-block {
  margin: 0.85em 0;
  width: 100%;
  max-width: 100%;
  box-sizing: border-box;
  border-radius: 12px;
  overflow: hidden;
  border: 1px solid #d0d7de;
  background: #f6f8fa;
  display: flex;
  flex-direction: column;
  box-shadow: inset 0 0 0 1px rgba(255,255,255,0.6), 0 1px 2px rgba(31,35,40,0.04);
}

.hljs-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 5px 12px;
  background: #eaeef2;
  border-bottom: 1px solid #d0d7de;
  z-index: 1;
}

.hljs-lang {
  font-size: 11px;
  color: #6e7781;
  font-family: 'SF Mono', ui-monospace, monospace;
  text-transform: lowercase;
  letter-spacing: 0.02em;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hljs-copy {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  color: #6e7781;
  background: transparent;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: color 0.15s, background 0.15s, transform 0.1s;
  position: relative;
}

.hljs-copy:hover {
  color: #24292f;
  background: rgba(175,184,193,0.2);
}

.hljs-copy:active {
  transform: scale(0.92);
}

.hljs-copy.is-copied {
  color: #1a7f37;
  background: rgba(34,197,94,0.12);
}

.hljs-copy-icon {
  display: block;
  width: 14px;
  height: 14px;
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.hljs-copy-icon.is-check {
  position: absolute;
  opacity: 0;
  transform: scale(0.6) rotate(-10deg);
}

.hljs-copy.is-copied .hljs-copy-icon:not(.is-check) {
  opacity: 0;
  transform: scale(0.8);
}

.hljs-copy.is-copied .hljs-copy-icon.is-check {
  opacity: 1;
  transform: scale(1) rotate(0deg);
}

.hljs-pre {
  margin: 0;
  padding: 0;
  flex: 1 1 auto;
  overflow: visible;
  background: transparent;
}

.hljs-pre::-webkit-scrollbar {
  display: none;
}

.hljs-block code.hljs {
  display: block;
  padding: 12px 14px 14px;
  overflow: visible;
  font-size: 13px;
  line-height: 1.65;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', ui-monospace, monospace;
  background: transparent !important;
  color: #24292f;
}

/* 浅色语法高亮 - GitHub light 风格 */
.hljs-block .hljs-comment,
.hljs-block .hljs-quote {
  color: #6e7781;
  font-style: italic;
}

.hljs-block .hljs-keyword,
.hljs-block .hljs-selector-tag,
.hljs-block .hljs-addition {
  color: #cf222e;
}

.hljs-block .hljs-built_in,
.hljs-block .hljs-type,
.hljs-block .hljs-class .hljs-title {
  color: #0550ae;
}

.hljs-block .hljs-number,
.hljs-block .hljs-literal {
  color: #0550ae;
}

.hljs-block .hljs-string,
.hljs-block .hljs-doctag,
.hljs-block .hljs-regexp,
.hljs-block .hljs-meta .hljs-meta-string {
  color: #0a3069;
}

.hljs-block .hljs-title,
.hljs-block .hljs-section,
.hljs-block .hljs-name,
.hljs-block .hljs-selector-id,
.hljs-block .hljs-selector-class {
  color: #8250df;
}

.hljs-block .hljs-attr,
.hljs-block .hljs-attribute,
.hljs-block .hljs-variable,
.hljs-block .hljs-template-variable {
  color: #953800;
}

.hljs-block .hljs-params {
  color: #24292f;
}

.hljs-block .hljs-symbol,
.hljs-block .hljs-bullet,
.hljs-block .hljs-link,
.hljs-block .hljs-meta {
  color: #1f2328;
}

.hljs-block .hljs-deletion {
  color: #82071e;
}

.hljs-block .hljs-emphasis {
  font-style: italic;
}

.hljs-block .hljs-strong {
  font-weight: 700;
}

/* 深色模式 */
.dark .hljs-block {
  border-color: #30363d;
  background: #0f172a;
  box-shadow: none;
}

.dark .hljs-header {
  background: #161b22;
  border-bottom-color: #30363d;
}

.dark .hljs-lang {
  color: #7d8590;
}

.dark .hljs-copy {
  color: #64748b;
}

.dark .hljs-copy:hover {
  color: #cbd5e1;
  background: rgba(148, 163, 184, 0.12);
}

.dark .hljs-copy.is-copied {
  color: #4ade80;
  background: rgba(34, 197, 94, 0.16);
}

.dark .hljs-block code.hljs {
  color: #e2e8f0;
}

.dark .hljs-block .hljs-comment,
.dark .hljs-block .hljs-quote {
  color: #64748b;
}

.dark .hljs-block .hljs-keyword,
.dark .hljs-block .hljs-selector-tag,
.dark .hljs-block .hljs-addition {
  color: #fb923c;
}

.dark .hljs-block .hljs-built_in,
.dark .hljs-block .hljs-type,
.dark .hljs-block .hljs-title,
.dark .hljs-block .hljs-name {
  color: #60a5fa;
}

.dark .hljs-block .hljs-string,
.dark .hljs-block .hljs-doctag,
.dark .hljs-block .hljs-regexp {
  color: #34d399;
}

.dark .hljs-block .hljs-number,
.dark .hljs-block .hljs-literal {
  color: #c084fc;
}

.dark .hljs-block .hljs-attr,
.dark .hljs-block .hljs-variable {
  color: #fbbf24;
}

.dark .hljs-block .hljs-params {
  color: #cbd5e1;
}

.md-body blockquote {
  margin: 0.8em 0;
  padding: 10px 16px;
  border-left: 3px solid #d8c6ad;
  background: #fbf7ef;
  border-radius: 0 6px 6px 0;
  color: #4b5563;
}

.md-body blockquote p {
  margin: 0;
}

.md-body ul,
.md-body ol {
  margin: 0.5em 0;
  padding-left: 1.6em;
}

.md-body li {
  margin: 0.25em 0;
}

.md-body li::marker {
  color: #a3917a;
}

.md-body ul>li {
  list-style-type: disc;
}

.md-body ol>li {
  list-style-type: decimal;
}

.md-body .md-table-wrap {
  display: block;
  width: 100%;
  max-width: 100%;
  overflow-x: auto;
  overflow-y: hidden;
  margin: 0.85em 0;
  -webkit-overflow-scrolling: touch;
  overflow-anchor: none;
}

.md-body table {
  border-collapse: collapse;
  margin: 0;
  font-size: 14px;
  line-height: 1.45;
  display: table;
  width: max-content;
  min-width: 100%;
  max-width: none;
  table-layout: auto;
  background: transparent;
}

.md-body thead {
  border: none;
}

.md-body th {
  background: #eeeeed;
  font-weight: 700;
  color: #111827;
  padding: 6px 8px;
  border: 2px solid #ffffff;
  text-align: left;
  vertical-align: middle;
}

.md-body td {
  background: #f3f3f2;
  padding: 6px 8px;
  border: 2px solid #ffffff;
  color: #1f2937;
  vertical-align: middle;
}

.dark .md-body th {
  background: #30343b;
  color: #f3f4f6;
}

.dark .md-body thead {
  border: none;
}

.dark .md-body td {
  background: #282c33;
  color: #d1d5db;
}

.dark .md-body th,
.dark .md-body td {
  border-color: #111827;
}

.md-body hr {
  border: none;
  border-top: 1px solid #e5e7eb;
  margin: 1.2em 0;
}

.md-body a {
  color: #315f8f;
  text-decoration: none;
}

.md-body a:hover {
  text-decoration: underline;
}

.md-body img {
  max-width: 100%;
  border-radius: 6px;
}

.md-body .katex-display {
  margin: 0.8em 0;
  overflow-x: auto;
  overflow-y: hidden;
}

.md-body .katex {
  font-size: 1em;
}

.md-body details {
  margin: 0.8em 0;
  border: 1px solid #e7ded2;
  border-radius: 6px;
  overflow: hidden;
}

.md-body summary {
  padding: 8px 14px;
  cursor: pointer;
  font-weight: 500;
  background: #fbf7ef;
  user-select: none;
  list-style: revert;
}

.md-body summary:hover {
  background: #f6f1e9;
}

.md-body .details-body {
  padding: 8px 14px;
}

.md-body .details-body>*:first-child {
  margin-top: 0;
}

.md-body .details-body>*:last-child {
  margin-bottom: 0;
}

.dark .md-body h1,
.dark .md-body h2,
.dark .md-body h3,
.dark .md-body h4,
.dark .md-body h5,
.dark .md-body h6,
.dark .md-body strong {
  color: #f3f4f6;
}

.dark .md-body h2 {
  border-bottom-color: #3f4652;
}

.dark .md-body code:not(.hljs) {
  background: #252b34;
  color: #e5e7eb;
  border-color: #3f4652;
  box-shadow: none;
}

.dark .md-body blockquote {
  border-left-color: #4b5563;
  background: #1f2937;
  color: #d1d5db;
}

.dark .md-body details {
  border-color: #3f4652;
}

.dark .md-body summary {
  background: #1f2937;
}

.dark .md-body summary:hover {
  background: #252b34;
}

.dark .md-body a {
  color: #93c5fd;
}

.dark .md-body hr {
  border-top-color: #3f4652;
}
</style>
