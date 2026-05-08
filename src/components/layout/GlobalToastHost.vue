<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { Card, CardContent } from '@/components/ui/card';
import { NOVA_TOAST_EVENT, type ToastPayload } from '../../lib/toast';

type ToastItem = {
  id: number;
  message: string;
  variant: 'error' | 'success' | 'info' | 'warning';
  createdAt: number;
};

const toasts = ref<ToastItem[]>([]);

const isDuplicateToast = (message: string): boolean => {
  const now = Date.now();
  return toasts.value.some((t) => t.message === message && now - t.createdAt < 1200);
};

const onToast = (event: Event) => {
  const customEvent = event as CustomEvent<ToastPayload>;
  const payload = customEvent.detail;
  if (!payload?.message) {
    return;
  }
  if (isDuplicateToast(payload.message)) {
    return;
  }

  const id = Date.now() + Math.floor(Math.random() * 1000);
  const createdAt = Date.now();
  toasts.value.push({
    id,
    message: payload.message,
    variant: payload.variant ?? 'info',
    createdAt,
  });

  window.setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }, 4200);
};

onMounted(() => {
  window.addEventListener(NOVA_TOAST_EVENT, onToast as EventListener);
});

onUnmounted(() => {
  window.removeEventListener(NOVA_TOAST_EVENT, onToast as EventListener);
});
</script>

<template>
  <TransitionGroup
    name="global-toast"
    tag="div"
    class="fixed top-5 right-5 z-[120] flex flex-col gap-2 pointer-events-none"
  >
    <Card
      v-for="toast in toasts"
      :key="toast.id"
      class="min-w-[280px] max-w-[420px] border py-0 shadow-[0_8px_20px_rgba(0,0,0,0.12)] pointer-events-auto"
      :class="{
        'bg-[#fff4f4] dark:bg-[#3a2222] border-[#f2c9c9] dark:border-[#6a3535] text-[#9f2f2f] dark:text-[#ffb3b3]': toast.variant === 'error',
        'bg-[#f2fbf4] dark:bg-[#1f3325] border-[#cde8d3] dark:border-[#3a6b48] text-[#1f6a34] dark:text-[#9ae2ad]': toast.variant === 'success',
        'bg-[#f3f7ff] dark:bg-[#202a3a] border-[#d2def8] dark:border-[#3a4d74] text-[#2f4e91] dark:text-[#a4c0ff]': toast.variant === 'info'
      }"
    >
      <CardContent class="px-4 py-3 text-[13px] leading-relaxed">{{ toast.message }}</CardContent>
    </Card>
  </TransitionGroup>
</template>

<style scoped>
.global-toast-enter-active,
.global-toast-leave-active {
  transition: all 0.22s ease;
}

.global-toast-enter-from,
.global-toast-leave-to {
  opacity: 0;
  transform: translateY(-8px) translateX(8px);
}
</style>
