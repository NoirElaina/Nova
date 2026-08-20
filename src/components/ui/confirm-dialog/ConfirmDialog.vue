<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import { Button } from '@/components/ui/button'

const props = withDefaults(defineProps<{
  modelValue: boolean
  title: string
  description?: string
  confirmText?: string
  cancelText?: string
  busy?: boolean
  destructive?: boolean
  /** 卡片宽度 class（默认 460px；内容较多的弹窗可传更宽的值，如 max-w-[720px]）。 */
  widthClass?: string
}>(), {
  description: '',
  confirmText: 'Confirm',
  cancelText: 'Cancel',
  busy: false,
  destructive: false,
  widthClass: 'max-w-[460px]',
})

const emit = defineEmits<{
  (e: 'update:modelValue', value: boolean): void
  (e: 'confirm'): void
}>()

const confirmVariant = computed(() => (props.destructive ? 'destructive' : 'default'))

const close = () => {
  if (props.busy) return
  emit('update:modelValue', false)
}

const handleConfirm = () => {
  if (props.busy) return
  emit('confirm')
}

const handleKeydown = (event: KeyboardEvent) => {
  if (!props.modelValue) return
  if (event.key === 'Escape') {
    event.preventDefault()
    close()
  }
}

onMounted(() => {
  window.addEventListener('keydown', handleKeydown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="confirm-backdrop">
      <div
        v-if="modelValue"
        class="fixed inset-0 z-[95] flex items-center justify-center bg-[rgba(15,18,24,0.36)] px-5 backdrop-blur-[4px]"
        @click.self="close"
      >
        <Transition name="confirm-card">
          <div
            v-if="modelValue"
            class="w-full rounded-[20px] border border-[#e5e7eb] bg-white p-6 shadow-[0_24px_70px_rgba(15,23,42,0.16)] dark:border-[#3a3a3a] dark:bg-[#242424]"
            :class="widthClass"
          >
            <div class="flex items-start justify-between gap-4">
              <div>
                <div class="text-[18px] font-semibold tracking-[-0.01em] text-[#111827] dark:text-[#f3f4f6]">
                  {{ title }}
                </div>
                <div
                  v-if="description"
                  class="mt-2.5 text-[13.5px] leading-6 text-[#64748b] dark:text-[#a3a3a3]"
                >
                  {{ description }}
                </div>
                <!-- 可选自定义内容：如新建场景的输入框，插在描述和按钮之间 -->
                <div v-if="$slots.default" class="mt-3">
                  <slot />
                </div>
              </div>

              <button
                type="button"
                class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[#94a3b8] transition-colors hover:bg-[#f1f5f9] hover:text-[#334155] dark:text-[#8b8b8b] dark:hover:bg-[#2f2f2f] dark:hover:text-[#e5e5e5]"
                :disabled="busy"
                @click="close"
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4">
                  <path d="M6 6l12 12M18 6L6 18" stroke-linecap="round" />
                </svg>
              </button>
            </div>

            <div class="mt-6 flex items-center justify-end gap-3">
              <Button
                variant="outline"
                size="sm"
                class="border-[#d8dee8] bg-white text-[#475569] hover:bg-[#f4f7fb] dark:border-[#3a3a3a] dark:bg-[#242424] dark:text-[#d7d7d7] dark:hover:bg-[#2d2d2d]"
                :disabled="busy"
                @click="close"
              >
                {{ cancelText }}
              </Button>
              <Button
                :variant="confirmVariant"
                size="sm"
                class="min-w-[96px]"
                :disabled="busy"
                @click="handleConfirm"
              >
                {{ confirmText }}
              </Button>
            </div>
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.confirm-backdrop-enter-active,
.confirm-backdrop-leave-active {
  transition: opacity 0.2s ease;
}

.confirm-backdrop-enter-from,
.confirm-backdrop-leave-to {
  opacity: 0;
}

.confirm-card-enter-active {
  transition: opacity 0.22s ease, transform 0.22s ease;
}

.confirm-card-leave-active {
  transition: opacity 0.16s ease, transform 0.16s ease;
}

.confirm-card-enter-from,
.confirm-card-leave-to {
  opacity: 0;
  transform: translateY(10px) scale(0.98);
}
</style>
