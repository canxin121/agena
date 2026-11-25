<script setup lang="ts">
import { computed } from 'vue'
import { PopoverAnchor, PopoverContent, PopoverPortal, PopoverRoot, type PopoverRootEmits } from 'radix-vue'
import { focusDialogOnOpen, preserveImeEscape } from '@/lib/keyboardNavigation'
import { resolveOverlayAnchor, type OverlayAnchor } from '@/lib/overlayAnchor'

defineOptions({ inheritAttrs: false })

const props = withDefaults(
  defineProps<{
    open?: boolean
    anchor?: OverlayAnchor
    portalled?: boolean
    layer?: number
    initialFocus?: string
  }>(),
  { open: undefined, portalled: true, layer: 80 },
)
const emit = defineEmits<PopoverRootEmits>()

const measuredAnchor = computed(() => {
  const anchor = resolveOverlayAnchor(props.anchor)
  if (!anchor || anchor instanceof HTMLElement) return anchor ?? undefined
  return { getBoundingClientRect: () => anchor.getBoundingClientRect() ?? new DOMRect() }
})
</script>

<template>
  <PopoverRoot :open="open" @update:open="emit('update:open', $event)">
    <slot name="trigger" />
    <PopoverAnchor v-if="measuredAnchor" :element="measuredAnchor" class="absolute h-0 w-0" aria-hidden="true" />
    <PopoverPortal :disabled="!portalled">
      <PopoverContent
        v-bind="$attrs"
        data-keyboard-overlay="true"
        class="oc-overlay-panel max-h-[var(--radix-popover-content-available-height)]"
        :style="{ zIndex: layer }"
        @keydown.capture="preserveImeEscape"
        @open-auto-focus="!$event.defaultPrevented && focusDialogOnOpen($event, initialFocus)"
      >
        <slot />
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
