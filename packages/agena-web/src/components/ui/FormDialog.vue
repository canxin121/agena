<script setup lang="ts">
import { computed } from 'vue'
import Dialog from '@/components/ui/Dialog.vue'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    description?: string
    maxWidth?: string
    mobileTitle?: string
    mobileFillViewport?: boolean
    isCompactTouch?: boolean
    initialFocus?: string
  }>(),
  { isCompactTouch: undefined },
)
const emit = defineEmits<{ close: []; 'update:open': [open: boolean] }>()

const ui = useUiStore()
const compact = computed(() => props.isCompactTouch ?? ui.isCompactTouch)
</script>

<template>
  <Dialog
    :open="open"
    :title="(compact && mobileTitle) || title"
    :description="description"
    :max-width="maxWidth"
    :is-compact-touch="compact"
    :mobile-placement="mobileFillViewport ? 'fill' : 'top'"
    :initial-focus="initialFocus"
    @close="emit('close')"
    @update:open="emit('update:open', $event)"
  >
    <slot />
    <template v-if="$slots.footer" #footer>
      <slot name="footer" />
    </template>
  </Dialog>
</template>
