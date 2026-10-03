<script setup lang="ts">
import { DialogRoot, DialogContent, DialogOverlay, DialogPortal } from 'radix-vue'
import { computed } from 'vue'
import { cn } from '@/lib/utils'
import DialogHeader from '@/components/ui/DialogHeader.vue'
import { useUiStore } from '@/stores/ui'

const props = defineProps<{
  open: boolean
  title?: string
  description?: string
  maxWidth?: string
  mobileFullscreen?: boolean
  /** The child owns scrolling (editors, plan viewers, split panes). */
  bodyScroll?: boolean
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'update:open', value: boolean): void
}>()

function onUpdateOpen(open: boolean) {
  if (!open) emit('close')
  emit('update:open', open)
}

const ui = useUiStore()
const useMobileFullscreen = computed(() => Boolean(props.mobileFullscreen && ui.isCompactTouch))
const contentClass = computed(() =>
  cn(
    'fixed z-[71] pointer-events-auto flex min-w-0 flex-col overflow-hidden bg-background shadow-xl duration-150 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
    useMobileFullscreen.value
      ? 'inset-0 h-[100dvh] w-screen pt-[var(--oc-safe-area-top,0px)] pr-[var(--oc-safe-area-right,0px)] pb-[var(--oc-safe-area-bottom,0px)] pl-[var(--oc-safe-area-left,0px)]'
      : 'left-1/2 top-1/2 w-[calc(100vw-1rem)] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border/70 max-h-[calc(100dvh-1rem)] sm:max-h-[calc(100dvh-3rem)]',
    useMobileFullscreen.value ? 'max-w-none' : props.maxWidth || 'max-w-lg',
  ),
)
</script>

<template>
  <DialogRoot :open="open" @update:open="onUpdateOpen">
    <DialogPortal>
      <DialogOverlay
        class="fixed inset-0 z-[70] pointer-events-auto bg-black/55 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
      />
      <DialogContent :class="contentClass">
        <DialogHeader :title="title" :description="description" @close="onUpdateOpen(false)" />
        <div
          class="min-h-0 min-w-0 flex-1 p-3 sm:p-4"
          :class="
            bodyScroll === false
              ? 'flex flex-col overflow-hidden'
              : 'overflow-y-auto overflow-x-hidden overscroll-contain'
          "
        >
          <slot />
        </div>
        <footer
          v-if="$slots.footer"
          class="flex shrink-0 flex-wrap items-center gap-2 border-t border-border/50 px-3 py-2 sm:px-4"
        >
          <slot name="footer" />
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
