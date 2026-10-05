<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { emitErrorToast } from '../../lib/toast';

type ApprovalPolicyValue = 'always_ask' | 'on_request' | 'never';

const props = defineProps<{
  settings: Record<string, any> | null;
}>();

const emit = defineEmits<{
  (e: 'update:settings', newSettings: Record<string, any>): void;
}>();

const policyMenuOpen = ref(false);
const policyButtonRef = ref<HTMLElement | null>(null);

const policyOptions: { value: ApprovalPolicyValue; label: string; desc: string }[] = [
  { value: 'always_ask', label: '每次都问', desc: '凡受控操作都弹审批（严格）' },
  { value: 'on_request', label: '仅风险操作询问', desc: 'Safe 直接放行，仅 Risky 弹审批（推荐）' },
  { value: 'never', label: '从不询问', desc: '除硬拒绝项外全部放行（慎用）' },
];

const approvalPolicy = computed<ApprovalPolicyValue>(() => {
  const v = props.settings?.approvalPolicy;
  return v === 'always_ask' || v === 'never' ? v : 'on_request';
});

const disclosureEnabled = computed(() => props.settings?.progressiveToolDisclosure !== false);

const policyShortLabel = computed(
  () =>
    ({
      always_ask: '每次询问',
      on_request: '仅风险',
      never: '从不询问',
    })[approvalPolicy.value],
);

const patchSettings = async (patch: Record<string, unknown>) => {
  try {
    const base = await invoke<Record<string, unknown>>('get_settings');
    const next = { ...base, ...patch };
    await invoke('save_settings', { settings: next });
    emit('update:settings', next);
    window.dispatchEvent(new CustomEvent('settings-updated'));
  } catch (error) {
    emitErrorToast('保存设置失败', error, 'policy-switcher');
  }
};

const setApprovalPolicy = (value: ApprovalPolicyValue) => {
  if (value === approvalPolicy.value) return;
  void patchSettings({ approvalPolicy: value });
};

const toggleDisclosure = () => {
  void patchSettings({ progressiveToolDisclosure: !disclosureEnabled.value });
};

const handleDocumentClick = (e: MouseEvent) => {
  if (!policyMenuOpen.value) return;
  const target = e.target as HTMLElement | null;
  if (policyButtonRef.value && target && policyButtonRef.value.contains(target)) return;
  const policyMenus = document.querySelectorAll('[data-policy-menu]');
  for (const menu of policyMenus) {
    if (target && menu.contains(target)) return;
  }
  policyMenuOpen.value = false;
};

onMounted(() => {
  document.addEventListener('click', handleDocumentClick);
});

onUnmounted(() => {
  document.removeEventListener('click', handleDocumentClick);
});
</script>

<template>
  <div class="relative shrink-0">
    <button
      ref="policyButtonRef"
      type="button"
      class="flex h-7 items-center gap-1 rounded-md border border-input bg-transparent px-2 text-xs text-muted-foreground transition-colors hover:bg-secondary/60"
      :class="{ 'bg-secondary/60': policyMenuOpen }"
      title="审批策略与工具披露快捷开关"
      @click="policyMenuOpen = !policyMenuOpen"
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
        stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      <span>{{ policyShortLabel }}</span>
    </button>

    <div
      v-if="policyMenuOpen"
      data-policy-menu
      class="absolute bottom-full left-0 z-50 mb-2 w-[280px] rounded-lg border border-border bg-popover p-2 shadow-lg"
    >
      <div class="px-1.5 pb-1 text-[11px] font-medium text-muted-foreground">审批策略</div>
      <button
        v-for="opt in policyOptions"
        :key="opt.value"
        type="button"
        class="flex w-full items-start gap-2 rounded-md px-1.5 py-1.5 text-left transition-colors hover:bg-secondary/80"
        @click="setApprovalPolicy(opt.value)"
      >
        <span
          class="mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border"
          :class="approvalPolicy === opt.value ? 'border-[#2563eb]' : 'border-muted-foreground/40'"
        >
          <span v-if="approvalPolicy === opt.value" class="h-1.5 w-1.5 rounded-full bg-[#2563eb]" />
        </span>
        <span class="min-w-0">
          <span class="block text-xs font-medium">{{ opt.label }}</span>
          <span class="block text-[11px] leading-snug text-muted-foreground">{{ opt.desc }}</span>
        </span>
      </button>

      <div class="my-1.5 border-t border-border" />

      <button
        type="button"
        class="flex w-full items-start gap-2 rounded-md px-1.5 py-1.5 text-left transition-colors hover:bg-secondary/80"
        @click="toggleDisclosure"
      >
        <span
          class="mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border"
          :class="disclosureEnabled ? 'border-[#2563eb] bg-[#2563eb]' : 'border-muted-foreground/40'"
        >
          <svg v-if="disclosureEnabled" width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="#fff"
            stroke-width="3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M20 6 9 17l-5-5" />
          </svg>
        </span>
        <span class="min-w-0">
          <span class="block text-xs font-medium">渐进式工具披露</span>
          <span class="block text-[11px] leading-snug text-muted-foreground">低频工具不进默认清单，由 LoadTool 按需加载，节省上下文</span>
        </span>
      </button>
    </div>
  </div>
</template>

