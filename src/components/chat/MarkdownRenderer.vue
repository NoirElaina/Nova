<script setup lang="ts">
import { computed } from 'vue'
import { Card, CardContent } from '@/components/ui/card'
import { renderMarkdown } from '@/lib/markdown-render'

const props = defineProps<{ content: string }>()

const rendered = computed(() => renderMarkdown(props.content || ''))
</script>

<template>
  <Card class="border-0 bg-transparent py-0 shadow-none">
    <CardContent class="px-0">
      <div class="md-body" v-html="rendered" />
    </CardContent>
  </Card>
</template>

<style>
@import 'highlight.js/styles/github-dark.css';
@import 'katex/dist/katex.min.css';

.md-body {
  font-size: 14px;
  line-height: 1.75;
  color: inherit;
  word-break: break-word;
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
  background: #f1f5f9;
  color: #1d4ed8;
  padding: 1px 5px;
  border-radius: 4px;
  border: 1px solid #e5e7eb;
}

.hljs-block {
  margin: 0.8em 0;
  border-radius: 10px;
  overflow: hidden;
  border: 1px solid #1f2937;
  background: #1e1e1e;
}

.hljs-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 14px;
  background: #2a2a2a;
  border-bottom: 1px solid #333;
}

.hljs-lang {
  font-size: 11px;
  color: #888;
  font-family: 'SF Mono', monospace;
  text-transform: lowercase;
}

.hljs-copy {
  font-size: 11px;
  color: #888;
  background: transparent;
  border: 1px solid #444;
  border-radius: 4px;
  padding: 2px 8px;
  cursor: pointer;
  transition: all 0.15s;
  font-family: system-ui, sans-serif;
}

.hljs-copy:hover {
  color: #ccc;
  border-color: #666;
}

.hljs-block code {
  display: block;
  padding: 14px 16px;
  overflow-x: auto;
  font-size: 13px;
  line-height: 1.6;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
}

.md-body blockquote {
  margin: 0.8em 0;
  padding: 10px 16px;
  border-left: 3px solid #cbd5e1;
  background: #f8fafc;
  border-radius: 0 6px 6px 0;
  color: #475569;
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
  color: #94a3b8;
}

.md-body ul>li {
  list-style-type: disc;
}

.md-body ol>li {
  list-style-type: decimal;
}

.md-body table {
  border-collapse: collapse;
  margin: 0.8em 0;
  font-size: 13px;
  display: block;
  overflow-x: auto;
  max-width: 100%;
}

.md-body th {
  background: #f8fafc;
  font-weight: 600;
  color: #111827;
  padding: 8px 12px;
  border: 1px solid #e5e7eb;
  text-align: left;
  white-space: nowrap;
}

.md-body td {
  padding: 7px 12px;
  border: 1px solid #e5e7eb;
  color: #334155;
  white-space: nowrap;
}

.md-body tr:nth-child(even) td {
  background: #f8fafc;
}

.md-body hr {
  border: none;
  border-top: 1px solid #e5e7eb;
  margin: 1.2em 0;
}

.md-body a {
  color: #2563eb;
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

/* details / summary */
.md-body details {
  margin: 0.8em 0;
  border: 1px solid #e5e7eb;
  border-radius: 6px;
  overflow: hidden;
}

.md-body summary {
  padding: 8px 14px;
  cursor: pointer;
  font-weight: 500;
  background: #f8fafc;
  user-select: none;
  list-style: revert;
}

.md-body summary:hover {
  background: #f1f5f9;
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
</style>
