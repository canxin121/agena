<script setup lang="ts">
import { DialogRoot, DialogContent, DialogOverlay, DialogPortal } from 'radix-vue'
import { computed, nextTick } from 'vue'
import { cn } from '@/lib/utils'
import DialogHeader from '@/components/ui/DialogHeader.vue'
import { useUiStore } from '@/stores/ui'
import { useDialogReturnFocus } from '@/composables/useDialogReturnFocus'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    description?: string
    maxWidth?: string
    mobileFullscreen?: boolean
    /** The child owns scrolling (editors, plan viewers, split panes). */
    bodyScroll?: boolean
    /** Explicit trigger for dialogs opened without a Radix DialogTrigger. */
    returnFocus?: HTMLElement | null
  }>(),
  {
    // Vue casts an absent Boolean prop to false. Scrolling must be opt-out;
    // otherwise ordinary dialogs silently clip everything below the viewport.
    bodyScroll: true,
  },
)

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'update:open', value: boolean): void
}>()

function onUpdateOpen(open: boolean) {
  if (!open) emit('close')
  emit('update:open', open)
}

function handleEscape(event: KeyboardEvent) {
  if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.altKey || event.metaKey) return
  const target = event.target
  // Portalled nested dialogs and pickers own their dismissal. A focused native
  // dialog handles its own Escape without waiting on the shared layer registry.
  if (!(target instanceof Element) || target.closest('[role="dialog"]') !== event.currentTarget) return
  event.preventDefault()
  event.stopPropagation()
  onUpdateOpen(false)
}

const ui = useUiStore()
const onCloseAutoFocus = useDialogReturnFocus(() => props.open)
function handleCloseAutoFocus(event: Event) {
  const target = props.returnFocus
  if (target?.isConnected && target.getClientRects().length && !target.closest('[inert]')) {
    event.preventDefault()
    void nextTick(() => target.focus({ preventScroll: true }))
    return
  }
  onCloseAutoFocus(event)
}
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
      <DialogContent :class="contentClass" @close-auto-focus="handleCloseAutoFocus" @keydown.esc="handleEscape">
        <DialogHeader :title="title" :description="description" @close="onUpdateOpen(false)" />
        <div
          data-dialog-body
          :tabindex="bodyScroll === false ? undefined : 0"
          :role="bodyScroll === false ? undefined : 'region'"
          :aria-label="title"
          class="min-h-0 min-w-0 flex-1 p-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:p-4"
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
