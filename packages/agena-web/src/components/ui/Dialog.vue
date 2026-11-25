<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import DialogHeader from '@/components/ui/DialogHeader.vue'
import ModalSurface from '@/components/ui/ModalSurface.vue'
import { useMobileSheetLayout, type MobileSheetPlacement } from '@/composables/useMobileSheetLayout'
import { cn } from '@/lib/utils'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    description?: string
    maxWidth?: string
    isCompactTouch?: boolean
    mobilePlacement?: MobileSheetPlacement
    mobileFullscreen?: boolean
    /** Leave scrolling to an editor or a split pane within the dialog. */
    bodyScroll?: boolean
    returnFocus?: HTMLElement | null
    initialFocus?: string
  }>(),
  { isCompactTouch: undefined, mobilePlacement: 'center', bodyScroll: true },
)

const emit = defineEmits<{ close: []; 'update:open': [open: boolean] }>()
const ui = useUiStore()
const { t } = useI18n()

function setOpen(open: boolean) {
  if (!open) emit('close')
  emit('update:open', open)
}

const compact = computed(() => props.isCompactTouch ?? ui.isCompactTouch)
const fullscreen = computed(() => compact.value && props.mobileFullscreen)
const sheet = computed(() => compact.value && !fullscreen.value)
const sheetStyle = useMobileSheetLayout(
  () => props.open && sheet.value,
  () => props.mobilePlacement,
)
const scrollBody = computed(() => props.bodyScroll && !(sheet.value && props.mobilePlacement === 'fill'))
const contentClass = computed(() =>
  cn(
    'oc-overlay-panel fixed flex min-w-0 flex-col overflow-hidden',
    fullscreen.value
      ? 'inset-0 h-[100dvh] w-full rounded-none border-0 pt-[var(--oc-safe-area-top,0px)] pe-[var(--oc-safe-area-right,0px)] pb-[var(--oc-safe-area-bottom,0px)] ps-[var(--oc-safe-area-left,0px)]'
      : '-translate-x-1/2',
    sheet.value
      ? props.mobilePlacement === 'center' && '-translate-y-1/2'
      : !fullscreen.value &&
          'left-1/2 top-1/2 w-[calc(100vw-1rem)] -translate-y-1/2 max-h-[calc(100dvh-1rem)] sm:max-h-[calc(100dvh-3rem)]',
    !fullscreen.value && (props.maxWidth || (sheet.value ? 'max-w-none' : 'max-w-lg')),
  ),
)
</script>

<template>
  <ModalSurface
    :open="open"
    :class="contentClass"
    :style="sheet ? sheetStyle : undefined"
    :initial-focus="initialFocus"
    :return-focus="returnFocus"
    v-bind="description ? {} : { 'aria-describedby': undefined }"
    @update:open="setOpen"
  >
    <DialogHeader :title="title" :description="description" @close="setOpen(false)" />
    <div
      data-dialog-body
      :tabindex="scrollBody ? 0 : undefined"
      :role="scrollBody ? 'region' : undefined"
      :aria-label="scrollBody ? title || t('common.options') : undefined"
      class="min-h-0 min-w-0 flex-1 p-3 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:p-4"
      :class="scrollBody ? 'overflow-y-auto overscroll-contain' : 'flex flex-col overflow-hidden'"
    >
      <slot />
    </div>
    <footer
      v-if="$slots.footer"
      class="flex shrink-0 flex-wrap items-center gap-2 border-t border-border/50 px-3 py-2.5 sm:px-4"
    >
      <slot name="footer" />
    </footer>
  </ModalSurface>
</template>
