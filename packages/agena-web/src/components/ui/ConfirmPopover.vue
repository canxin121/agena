<script setup lang="ts">
import { readViewportRect, useViewportObserver } from '@/composables/useViewportObserver'
import { useMobileSheetLayout } from '@/composables/useMobileSheetLayout'
import { createReusableTemplate } from '@vueuse/core'
import { computed, onBeforeUnmount, ref, useId, useSlots, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { DialogDescription, DialogTitle, DialogTrigger, PopoverTrigger } from 'radix-vue'
import { RiLoader4Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import ModalSurface from '@/components/ui/ModalSurface.vue'
import PopoverSurface from '@/components/ui/PopoverSurface.vue'
import { lockListItemActionsForAnchor } from '@/components/ui/listItemActionVisibilityLock'
import { handleOptionKeydown } from '@/lib/keyboardNavigation'
import { cn } from '@/lib/utils'
import { useUiStore } from '@/stores/ui'
import { useDialogReturnFocus } from '@/composables/useDialogReturnFocus'

const props = withDefaults(
  defineProps<{
    open?: boolean | null
    title?: string
    description?: string
    confirmText?: string
    cancelText?: string
    variant?: 'default' | 'destructive'
    anchorToCursor?: boolean
    forceDialog?: boolean
    maxWidth?: string
    closeOnConfirm?: boolean
    closeOnCancel?: boolean
    confirmDisabled?: boolean
    cancelDisabled?: boolean
    busy?: boolean
    /** Set to false when the actions slot supplies the confirmation controls. */
    showDefaultActions?: boolean
  }>(),
  {
    open: null,
    confirmText: '',
    cancelText: '',
    variant: 'default',
    anchorToCursor: true,
    forceDialog: false,
    maxWidth: 'max-w-[calc(100vw-2rem)] sm:max-w-sm',
    closeOnConfirm: true,
    closeOnCancel: true,
    confirmDisabled: false,
    cancelDisabled: false,
    busy: false,
    showDefaultActions: true,
  },
)

const emit = defineEmits<{
  (e: 'confirm'): void
  (e: 'cancel'): void
  (e: 'update:open', value: boolean): void
}>()

const ui = useUiStore()
const slots = useSlots()
const { t } = useI18n()
const [DefineConfirmBody, ConfirmBody] = createReusableTemplate()
const contentId = useId()
const titleId = `confirmation-${contentId}-title`
const descriptionId = `confirmation-${contentId}-description`

const hasTrigger = computed(() => Boolean(slots.default))
const useDialog = computed(() => props.forceDialog || ui.isCompactTouch || !hasTrigger.value)

const openInternal = ref(false)
const isOpenControlled = computed(() => props.open !== null && props.open !== undefined)
const openModel = computed({
  get: () => (isOpenControlled.value ? (props.open as boolean) : openInternal.value),
  set: (next: boolean) => {
    if (!isOpenControlled.value) openInternal.value = next
    emit('update:open', next)
  },
})
const onCloseAutoFocus = useDialogReturnFocus(() => openModel.value)

const confirmButtonVariant = computed(() => (props.variant === 'destructive' ? 'destructive' : 'default'))
const confirmButtonDisabled = computed(() => props.confirmDisabled || props.busy)
const dialogStyle = useMobileSheetLayout(() => openModel.value && useDialog.value && ui.isCompactTouch)
const initialAction = computed(() =>
  props.variant === 'destructive' ? '[data-confirm-cancel]:not(:disabled)' : '[data-confirm-submit]:not(:disabled)',
)
const dialogContentClass = computed(() =>
  cn(
    'oc-overlay-panel fixed min-w-0 -translate-x-1/2 -translate-y-1/2 overflow-y-auto overscroll-contain',
    !ui.isCompactTouch && 'left-1/2 top-1/2 max-h-[calc(100dvh-3rem)] w-[calc(100vw-2rem)]',
    props.maxWidth,
  ),
)

const effectiveConfirmText = computed(() => {
  const raw = String(props.confirmText || '').trim()
  return raw || t('common.confirm')
})

const effectiveCancelText = computed(() => {
  const raw = String(props.cancelText || '').trim()
  return raw || t('common.cancel')
})

const anchorRect = ref<DOMRect | null>(null)
const triggerEl = ref<HTMLElement | null>(null)
const virtualRef = computed(() => {
  // Keep the custom anchor mounted when focus supplies the first rectangle.
  const rect = anchorRect.value
  return {
    getBoundingClientRect: () => rect ?? new DOMRect(),
  }
})

let releaseActionLock: (() => void) | null = null

function clearActionLock() {
  if (!releaseActionLock) return
  releaseActionLock()
  releaseActionLock = null
}

function syncActionLock() {
  clearActionLock()
  if (!openModel.value || useDialog.value) return
  releaseActionLock = lockListItemActionsForAnchor(triggerEl.value)
}

const POPOVER_EDGE_MARGIN_PX = 10
const POPOVER_MIN_HEIGHT_PX = 140
const POPOVER_SIDE_OFFSET_PX = 5

const viewportVersion = ref(0)
useViewportObserver(
  () => openModel.value && !useDialog.value && Boolean(anchorRect.value),
  () => {
    viewportVersion.value += 1
  },
)

const popoverSide = computed<'top' | 'bottom'>(() => {
  if (useDialog.value || !openModel.value) return 'bottom'
  const rect = anchorRect.value
  // A closed/unanchored popover needs no geometry. Reading visualViewport
  // while hundreds of message controls mount forces layout between inserts.
  if (!rect) return 'bottom'
  void viewportVersion.value
  const { height: viewportHeight, top: viewportTop } = readViewportRect()
  if (!viewportHeight) return 'bottom'

  const viewportBottom = viewportTop + viewportHeight
  const minTop = viewportTop + POPOVER_EDGE_MARGIN_PX
  const maxBottom = viewportBottom - POPOVER_EDGE_MARGIN_PX

  const spaceBelow = maxBottom - rect.bottom - POPOVER_SIDE_OFFSET_PX
  const spaceAbove = rect.top - minTop - POPOVER_SIDE_OFFSET_PX

  if (spaceBelow < POPOVER_MIN_HEIGHT_PX && spaceAbove > spaceBelow) {
    return 'top'
  }

  return 'bottom'
})

function rememberTriggerEl(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return
  triggerEl.value = target
}

function setAnchorFromTrigger(target: EventTarget | null) {
  if (!props.anchorToCursor || useDialog.value) return
  if (!(target instanceof HTMLElement)) return
  anchorRect.value = target.getBoundingClientRect()
}

function setOpen(next: boolean) {
  openModel.value = next
}

function onTriggerPointerEnter(event: PointerEvent) {
  rememberTriggerEl(event.currentTarget)
  setAnchorFromTrigger(event.currentTarget)
}

function onTriggerFocus(event: FocusEvent) {
  rememberTriggerEl(event.currentTarget)
  setAnchorFromTrigger(event.currentTarget)
}

function onTriggerPointerDown(event: PointerEvent) {
  rememberTriggerEl(event.currentTarget)
  if (!props.anchorToCursor || useDialog.value) return
  // Anchor the desktop popover to cursor coordinates so it stays stable even
  // if the trigger element shifts due to hover state changes.
  anchorRect.value = new DOMRect(event.clientX, event.clientY)
}

watch(
  () => [openModel.value, useDialog.value, triggerEl.value] as const,
  () => {
    syncActionLock()
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  clearActionLock()
})

function onCancel() {
  emit('cancel')
  if (props.closeOnCancel) setOpen(false)
}

function onConfirm() {
  if (confirmButtonDisabled.value) return
  emit('confirm')
  if (props.closeOnConfirm) setOpen(false)
}
</script>

<template>
  <DefineConfirmBody>
    <div class="min-w-0 space-y-3" :class="useDialog ? 'p-4 sm:p-5' : 'p-3'">
      <div v-if="useDialog || title || description" class="space-y-1">
        <component
          :is="useDialog ? DialogTitle : 'h2'"
          v-if="useDialog || title"
          v-bind="useDialog ? {} : { id: titleId }"
          class="font-semibold [overflow-wrap:anywhere]"
          :class="title ? 'text-sm leading-5' : 'sr-only'"
        >
          {{ title || effectiveConfirmText }}
        </component>
        <component
          :is="useDialog ? DialogDescription : 'p'"
          v-if="description"
          v-bind="useDialog ? {} : { id: descriptionId }"
          class="text-xs leading-5 whitespace-pre-wrap text-muted-foreground [overflow-wrap:anywhere]"
        >
          {{ description }}
        </component>
      </div>

      <slot name="content" />
      <div v-if="$slots.actions" class="grid gap-2">
        <slot name="actions" />
      </div>
      <div v-if="showDefaultActions" class="flex flex-wrap items-center justify-end gap-2">
        <Button
          data-keyboard-option
          data-confirm-cancel
          type="button"
          variant="secondary"
          :size="useDialog ? 'sm' : 'xs'"
          :class="ui.isCompactTouch && 'min-h-10'"
          :disabled="cancelDisabled"
          @click="onCancel"
        >
          {{ effectiveCancelText }}
        </Button>
        <Button
          data-keyboard-option
          data-confirm-submit
          type="button"
          :size="useDialog ? 'sm' : 'xs'"
          :class="ui.isCompactTouch && 'min-h-10'"
          :variant="confirmButtonVariant"
          :disabled="confirmButtonDisabled"
          @click="onConfirm"
        >
          <RiLoader4Line v-if="busy" class="h-4 w-4 animate-spin" aria-hidden="true" />
          {{ effectiveConfirmText }}
        </Button>
      </div>
    </div>
  </DefineConfirmBody>

  <ModalSurface
    v-if="useDialog"
    :open="openModel"
    :layer="80"
    :class="dialogContentClass"
    :style="ui.isCompactTouch ? dialogStyle : undefined"
    :initial-focus="initialAction"
    v-bind="description ? {} : { 'aria-describedby': undefined }"
    @update:open="setOpen"
    @keydown="handleOptionKeydown($event, $event.currentTarget as HTMLElement)"
  >
    <template v-if="hasTrigger" #trigger>
      <DialogTrigger as-child>
        <slot />
      </DialogTrigger>
    </template>
    <ConfirmBody />
  </ModalSurface>

  <PopoverSurface
    v-else
    :open="openModel"
    :anchor="anchorToCursor ? virtualRef : undefined"
    :initial-focus="initialAction"
    :aria-labelledby="title ? titleId : undefined"
    :aria-label="title ? undefined : effectiveConfirmText"
    :aria-describedby="description ? descriptionId : undefined"
    class="oc-overlay-fade w-64 max-w-[calc(100vw-1rem)] overflow-y-auto overscroll-contain"
    :side="popoverSide"
    align="start"
    :side-offset="POPOVER_SIDE_OFFSET_PX"
    :collision-padding="POPOVER_EDGE_MARGIN_PX"
    sticky="always"
    update-position-strategy="always"
    :prioritize-position="true"
    @update:open="setOpen"
    @close-auto-focus="onCloseAutoFocus"
    @keydown="handleOptionKeydown($event, $event.currentTarget as HTMLElement)"
  >
    <template #trigger>
      <PopoverTrigger
        as-child
        @pointerenter="onTriggerPointerEnter"
        @focus="onTriggerFocus"
        @pointerdown="onTriggerPointerDown"
      >
        <slot />
      </PopoverTrigger>
    </template>
    <ConfirmBody />
  </PopoverSurface>
</template>
