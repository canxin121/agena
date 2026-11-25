<script setup lang="ts">
import { nextTick, type HTMLAttributes } from 'vue'
import { DialogContent, DialogOverlay, DialogPortal, DialogRoot } from 'radix-vue'
import { useDialogReturnFocus } from '@/composables/useDialogReturnFocus'
import { focusDialogOnOpen, preserveImeEscape } from '@/lib/keyboardNavigation'

defineOptions({ inheritAttrs: false })

const props = withDefaults(
  defineProps<{
    open: boolean
    layer?: number
    backdropClass?: HTMLAttributes['class']
    initialFocus?: string
    returnFocus?: HTMLElement | null
  }>(),
  { layer: 70, backdropClass: 'bg-black/55' },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
  'open-auto-focus': [event: Event]
  'close-auto-focus': [event: Event]
}>()

const restoreFocus = useDialogReturnFocus(() => props.open)

function onOpenAutoFocus(event: Event) {
  emit('open-auto-focus', event)
  if (!event.defaultPrevented) focusDialogOnOpen(event, props.initialFocus)
}

function onCloseAutoFocus(event: Event) {
  emit('close-auto-focus', event)
  if (event.defaultPrevented) return
  const target = props.returnFocus
  if (target?.isConnected && target.getClientRects().length && !target.closest('[inert]')) {
    event.preventDefault()
    void nextTick(() => {
      if (!props.open) target.focus({ preventScroll: true })
    })
    return
  }
  restoreFocus(event)
}
</script>

<template>
  <DialogRoot :open="open" @update:open="emit('update:open', $event)">
    <slot name="trigger" />
    <DialogPortal>
      <DialogOverlay
        class="pointer-events-auto fixed inset-0 oc-overlay-fade"
        :class="backdropClass"
        :style="{ zIndex: layer }"
      />
      <DialogContent
        v-bind="$attrs"
        aria-modal="true"
        class="pointer-events-auto outline-none oc-overlay-fade"
        :style="{ zIndex: layer }"
        @keydown.capture="preserveImeEscape"
        @open-auto-focus="onOpenAutoFocus"
        @close-auto-focus="onCloseAutoFocus"
      >
        <slot />
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
