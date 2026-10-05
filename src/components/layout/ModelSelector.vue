<script setup lang="ts">
import { computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { emitErrorToast } from '../../lib/toast';

const props = defineProps<{
  settings: Record<string, any> | null;
}>();

const emit = defineEmits<{
  (e: 'update:settings', newSettings: Record<string, any>): void;
}>();

const normalizeProviderKey = (provider: string) => (provider || '').trim().toLowerCase() || 'anthropic';

const ensureActiveProfile = () => {
  if (!props.settings) return null;
  const provider = normalizeProviderKey(props.settings.provider || 'anthropic');
  const profiles = props.settings.providerProfiles || {};
  return profiles[provider] || null;
};

type ProviderGroup = { key: string; label: string; models: string[] };

const providerGroups = computed<ProviderGroup[]>(() => {
  if (!props.settings) return [];
  const profiles = (props.settings.providerProfiles ?? {}) as Record<string, any>;
  const customModels = (props.settings.customModels ?? {}) as Record<string, string[]>;
  const groups: ProviderGroup[] = [];
  for (const key of Object.keys(profiles)) {
    const provider = normalizeProviderKey(key);
    const listed = customModels[provider];
    const profileModel = typeof profiles[key]?.model === 'string' ? profiles[key].model.trim() : '';
    const models = Array.isArray(listed) && listed.length > 0 ? listed : profileModel ? [profileModel] : [];
    if (models.length === 0) continue;
    const displayName =
      typeof profiles[key]?.displayName === 'string' ? profiles[key].displayName.trim() : '';
    groups.push({ key: provider, label: displayName || provider, models });
  }
  const active = normalizeProviderKey(props.settings.provider || 'anthropic');
  groups.sort((a, b) =>
    a.key === active ? -1 : b.key === active ? 1 : a.label.localeCompare(b.label),
  );
  return groups;
});

const hasAnyModel = computed(() => providerGroups.value.some((g) => g.models.length > 0));

const currentModel = computed(() => {
  const profile = ensureActiveProfile();
  return profile?.model || '';
});

const currentModelKey = computed(() => {
  const provider = normalizeProviderKey(props.settings?.provider || 'anthropic');
  return `${provider}::${currentModel.value}`;
});

const onModelKeyChange = async (value: unknown) => {
  if (typeof value !== 'string' || !props.settings) return;
  const sep = value.indexOf('::');
  if (sep <= 0) return;
  const provider = value.slice(0, sep);
  const model = value.slice(sep + 2);
  if (!provider || !model) return;
  try {
    const base = await invoke<Record<string, any>>('get_settings');
    const profiles = { ...(base.providerProfiles ?? {}) };
    profiles[provider] = { ...(profiles[provider] ?? {}), model };
    const next = { ...base, provider, providerProfiles: profiles };
    await invoke('save_settings', { settings: next });
    emit('update:settings', next);
    window.dispatchEvent(new CustomEvent('settings-updated'));
  } catch (error) {
    emitErrorToast('切换模型失败', error, 'model-switcher');
  }
};

defineExpose({
  currentModel,
});
</script>

<template>
  <div v-if="hasAnyModel && settings" class="flex min-w-0 shrink-0 items-center gap-1.5">
    <Select :model-value="currentModelKey" @update:model-value="onModelKeyChange">
      <SelectTrigger size="sm" class="w-[170px] max-w-[28vw] text-xs">
        <SelectValue placeholder="选择模型" />
      </SelectTrigger>
      <SelectContent class="max-h-[300px] text-xs">
        <SelectGroup v-for="group in providerGroups" :key="group.key">
          <SelectLabel class="text-muted-foreground">{{ group.label }}</SelectLabel>
          <SelectItem
            v-for="model in group.models"
            :key="`${group.key}::${model}`"
            :value="`${group.key}::${model}`"
          >
            {{ model }}
          </SelectItem>
        </SelectGroup>
      </SelectContent>
    </Select>
  </div>
</template>

