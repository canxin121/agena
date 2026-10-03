<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type CSSProperties } from 'vue'
import { DialogContent, DialogOverlay, DialogPortal, DialogRoot } from 'radix-vue'

import DialogHeader from '@/components/ui/DialogHeader.vue'
import { cn } from '@/lib/utils'
import { useUiStore } from '@/stores/ui'

const props = withDefaults(
  defineProps<{
    open: boolean
    title?: string
    description?: string
    maxWidth?: string
    mobileTitle?: string
    mobileFillViewport?: boolean
  }>(),
  {
    mobileTitle: '',
    mobileFillViewport: false,
  },
)

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'update:open', value: boolean): void
}>()

const ui = useUiStore()
const isMobileSheet = computed(() => Boolean(ui.isCompactTouch))

const MOBILE_SHEET_MARGIN_PX = 8

const mobileSheetStyle = ref<CSSProperties>({
  top: `${MOBILE_SHEET_MARGIN_PX}px`,
  left: '50%',
  width: `calc(100% - ${MOBILE_SHEET_MARGIN_PX * 2}px)`,
  maxHeight: `calc(100dvh - ${MOBILE_SHEET_MARGIN_PX * 2}px)`,
  '--oc-form-dialog-mobile-max-height': `calc(100dvh - ${MOBILE_SHEET_MARGIN_PX * 2}px)`,
})

const desktopContentClass = computed(() =>
  cn(
    'fixed left-[50%] top-[50%] z-[71] pointer-events-auto flex w-[calc(100vw-1rem)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-lg border border-border/70 bg-background shadow-xl duration-200 max-h-[calc(100dvh-1rem)] data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[state=closed]:slide-out-to-left-1/2 data-[state=closed]:slide-out-to-top-[48%] data-[state=open]:slide-in-from-left-1/2 data-[state=open]:slide-in-from-top-[48%]',
    props.maxWidth || 'max-w-lg',
  ),
)

const mobileContentClass = computed(() =>
  cn(
    'fixed z-[71] pointer-events-auto flex -translate-x-1/2 flex-col overflow-hidden rounded-lg border border-border/70 bg-background shadow-xl duration-200 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
    props.maxWidth || 'max-w-none',
  ),
)

const contentClass = computed(() => (isMobileSheet.value ? mobileContentClass.value : desktopContentClass.value))
const contentStyle = computed<CSSProperties | undefined>(() =>
  isMobileSheet.value ? mobileSheetStyle.value : undefined,
)
const sheetTitle = computed(() => (isMobileSheet.value && props.mobileTitle ? props.mobileTitle : props.title || ''))
const contentBodyClass = computed(() =>
  cn(
    'min-h-0 min-w-0 flex-1 p-3 sm:p-4 overscroll-contain',
    isMobileSheet.value && props.mobileFillViewport ? 'overflow-hidden' : 'overflow-auto',
  ),
)

let mobileViewportEventsBound = false

function close() {
  emit('close')
  emit('update:open', false)
}

function onUpdateOpen(next: boolean) {
  if (!next) emit('close')
  emit('update:open', next)
}

function cssVarPx(name: string, fallback: number): number {
  if (typeof window === 'undefined' || typeof document === 'undefined') return fallback
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name)
  const parsed = Number.parseFloat(String(raw || '').trim())
  return Number.isFinite(parsed) ? parsed : fallback
}

function resolveViewportHeight(): number {
  if (typeof window === 'undefined') return 0
  const vvHeight = window.visualViewport?.height
  if (typeof vvHeight === 'number' && Number.isFinite(vvHeight) && vvHeight > 0) {
    return vvHeight
  }
  return window.innerHeight
}

function resolveViewportWidth(): number {
  if (typeof window === 'undefined') return 0
  const vvWidth = window.visualViewport?.width
  if (typeof vvWidth === 'number' && Number.isFinite(vvWidth) && vvWidth > 0) {
    return vvWidth
  }
  return window.innerWidth
}

function resolveViewportLeft(): number {
  if (typeof window === 'undefined') return 0
  const vvLeft = window.visualViewport?.offsetLeft
  if (typeof vvLeft === 'number' && Number.isFinite(vvLeft) && vvLeft > 0) {
    return vvLeft
  }
  return 0
}

function resolveViewportTop(): number {
  if (typeof window === 'undefined') return 0
  const vvTop = window.visualViewport?.offsetTop
  if (typeof vvTop === 'number' && Number.isFinite(vvTop) && vvTop > 0) {
    return vvTop
  }
  return 0
}

function syncMobileSheetPosition() {
  if (!props.open || !isMobileSheet.value) return
  if (typeof window === 'undefined' || typeof document === 'undefined') return

  const viewportHeight = resolveViewportHeight()
  if (!viewportHeight) return
  const viewportTop = resolveViewportTop()
  const viewportWidth = resolveViewportWidth()
  const viewportLeft = resolveViewportLeft()

  const safeTop = cssVarPx('--oc-safe-area-top', 0)
  const safeBottom = cssVarPx('--oc-safe-area-bottom', 0)
  const safeLeft = cssVarPx('--oc-safe-area-left', 0)
  const safeRight = cssVarPx('--oc-safe-area-right', 0)

  const topInset = viewportTop + safeTop + MOBILE_SHEET_MARGIN_PX
  const bottomEdge = viewportTop + viewportHeight - safeBottom - MOBILE_SHEET_MARGIN_PX
  const maxHeight = Math.max(0, bottomEdge - topInset)
  const usableWidth = Math.max(0, viewportWidth - safeLeft - safeRight)
  const panelWidth = Math.max(0, usableWidth - MOBILE_SHEET_MARGIN_PX * 2)
  const panelCenter = viewportLeft + safeLeft + usableWidth / 2

  const nextStyle: CSSProperties = {
    top: `${Math.round(topInset)}px`,
    left: `${Math.round(panelCenter)}px`,
    width: `${Math.round(panelWidth)}px`,
    maxHeight: `${Math.round(maxHeight)}px`,
    '--oc-form-dialog-mobile-max-height': `${Math.round(maxHeight)}px`,
  }

  if (props.mobileFillViewport) {
    nextStyle.height = `${Math.round(maxHeight)}px`
  }

  mobileSheetStyle.value = nextStyle
}

function onMobileViewportChange() {
  syncMobileSheetPosition()
}

function bindMobileViewportEvents() {
  if (mobileViewportEventsBound || typeof window === 'undefined') return
  window.addEventListener('resize', onMobileViewportChange)
  window.addEventListener('orientationchange', onMobileViewportChange)
  window.visualViewport?.addEventListener('resize', onMobileViewportChange)
  window.visualViewport?.addEventListener('scroll', onMobileViewportChange)
  mobileViewportEventsBound = true
}

function unbindMobileViewportEvents() {
  if (!mobileViewportEventsBound || typeof window === 'undefined') return
  window.removeEventListener('resize', onMobileViewportChange)
  window.removeEventListener('orientationchange', onMobileViewportChange)
  window.visualViewport?.removeEventListener('resize', onMobileViewportChange)
  window.visualViewport?.removeEventListener('scroll', onMobileViewportChange)
  mobileViewportEventsBound = false
}

watch(
  () => props.open,
  (open) => {
    if (open && isMobileSheet.value) {
      bindMobileViewportEvents()
      syncMobileSheetPosition()
      return
    }

    unbindMobileViewportEvents()
  },
  { immediate: true },
)

watch(
  () => isMobileSheet.value,
  (mobile) => {
    if (!props.open) return
    if (mobile) {
      bindMobileViewportEvents()
      syncMobileSheetPosition()
      return
    }

    unbindMobileViewportEvents()
  },
)

onBeforeUnmount(() => {
  unbindMobileViewportEvents()
})
</script>

<template>
  <DialogRoot :open="open" @update:open="onUpdateOpen">
    <DialogPortal>
      <DialogOverlay
        class="fixed inset-0 z-[70] pointer-events-auto bg-black/55 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
      />
      <DialogContent :class="contentClass" :style="contentStyle">
        <DialogHeader :title="sheetTitle" :description="description" @close="close" />

        <div :class="contentBodyClass">
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
