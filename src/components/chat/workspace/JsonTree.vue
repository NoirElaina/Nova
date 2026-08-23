<script setup lang="ts">
import { computed, ref } from 'vue';

/**
 * 递归 JSON 树查看器：一行一个 key，点击展开/收起 value。
 * - 对象/数组：可折叠容器，标题行显示 `{N}` / `[N]` 概要；
 * - 基础类型：按类型着色展示；超长字符串点击切换完整/截断。
 */
const props = withDefaults(
  defineProps<{
    /** 键名（顶层节点可省略）。 */
    name?: string | null;
    data: unknown;
    depth?: number;
    /** 自动展开到的深度（0=只展开顶层）。 */
    defaultExpandDepth?: number;
  }>(),
  {
    name: null,
    depth: 0,
    defaultExpandDepth: 1,
  },
);

const expanded = ref(props.depth < props.defaultExpandDepth);
const showFullString = ref(false);

const kind = computed(() => {
  if (props.data === null || props.data === undefined) return 'null';
  if (Array.isArray(props.data)) return 'array';
  return typeof props.data; // 'object' | 'string' | 'number' | 'boolean'
});

const isContainer = computed(() => kind.value === 'object' || kind.value === 'array');

const entries = computed<Array<readonly [string, unknown]>>(() => {
  if (kind.value === 'array') {
    return (props.data as unknown[]).map((value, index) => [String(index), value] as const);
  }
  if (kind.value === 'object') {
    return Object.entries(props.data as Record<string, unknown>);
  }
  return [];
});

/** 容器概要：{5} / [12]。 */
const summary = computed(() =>
  kind.value === 'array'
    ? `[${entries.value.length}]`
    : `{${entries.value.length}}`,
);

const STRING_LIMIT = 300;
const stringValue = computed(() => {
  if (kind.value === 'string') return props.data as string;
  return JSON.stringify(props.data);
});
const isLongString = computed(() => stringValue.value.length > STRING_LIMIT);
const displayedString = computed(() =>
  showFullString.value || !isLongString.value
    ? stringValue.value
    : `${stringValue.value.slice(0, STRING_LIMIT)}…（${stringValue.value.length.toLocaleString()} 字符，点击展开）`,
);

const valueClass = computed(() => {
  switch (kind.value) {
    case 'string':
      return 'text-emerald-700 dark:text-emerald-300';
    case 'number':
      return 'text-sky-700 dark:text-sky-300';
    case 'boolean':
      return 'text-violet-700 dark:text-violet-300';
    default:
      return 'text-[#94a3b8] dark:text-[#71717a]';
  }
});

const toggle = () => {
  expanded.value = !expanded.value;
};

const toggleString = () => {
  if (isLongString.value) {
    showFullString.value = !showFullString.value;
  }
};
</script>

<template>
  <div
    class="json-tree font-mono text-[10px] leading-relaxed"
    :class="depth > 0 ? 'border-l border-[#e5e7eb] pl-2.5 dark:border-[#3a3a3a]' : ''"
  >
    <!-- 容器节点：可折叠 -->
    <template v-if="isContainer && entries.length > 0">
      <button
        type="button"
        class="flex w-full items-center gap-1 rounded px-0.5 py-px text-left hover:bg-[#f3f4f6] dark:hover:bg-white/5"
        @click="toggle"
      >
        <svg
          width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6"
          stroke-linecap="round" stroke-linejoin="round"
          class="shrink-0 text-[#94a3b8] transition-transform"
          :class="expanded ? 'rotate-90' : ''"
        >
          <path d="M9 6l6 6-6 6" />
        </svg>
        <span v-if="name != null" class="shrink-0 text-[#7c3aed] dark:text-violet-300">{{ name }}:</span>
        <span class="text-[#94a3b8] dark:text-[#71717a]">{{ summary }}</span>
      </button>
      <div v-if="expanded" class="flex flex-col">
        <JsonTree
          v-for="[childKey, childValue] in entries"
          :key="childKey"
          :name="childKey"
          :data="childValue"
          :depth="depth + 1"
          :default-expand-depth="defaultExpandDepth"
        />
      </div>
    </template>

    <!-- 空容器 / 叶子节点：单行展示 -->
    <div v-else class="flex items-start gap-1 px-0.5 py-px">
      <span class="w-[9px] shrink-0" />
      <span v-if="name != null" class="shrink-0 text-[#7c3aed] dark:text-violet-300">{{ name }}:</span>
      <span v-if="isContainer" class="text-[#94a3b8] dark:text-[#71717a]">{{ summary }}</span>
      <span
        v-else
        class="whitespace-pre-wrap break-all"
        :class="[valueClass, isLongString ? 'cursor-pointer' : '']"
        :title="isLongString ? undefined : stringValue"
        @click="toggleString"
      >{{ kind === 'string' ? `"${displayedString}"` : displayedString }}</span>
    </div>
  </div>
</template>
