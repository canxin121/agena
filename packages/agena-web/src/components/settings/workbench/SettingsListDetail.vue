<script setup lang="ts">
import { onMounted, ref, watch, type HTMLAttributes } from 'vue'
import { RiArrowLeftLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'
import { cn } from '@/lib/utils'

const props = withDefaults(
  defineProps<{
    listLabel: string
    detailLabel: string
    backLabel?: string
    hasDetail?: boolean
    listBusy?: boolean
    detailLoading?: boolean
    listScrollClass?: HTMLAttributes['class']
    detailClass?: HTMLAttributes['class']
  }>(),
  { hasDetail: true },
)
const detailOpen = defineModel<boolean>('detailOpen', { default: false })
const list = ref<HTMLElement | null>(null)
const listBody = ref<HTMLElement | null>(null)
const detail = ref<HTMLElement | null>(null)
let lastListControl: HTMLElement | null = null

function rememberListControl(event: FocusEvent) {
  if (event.target instanceof HTMLElement) lastListControl = event.target
}

function focusList() {
  const region = list.value
  if (!region || !isVisibleKeyboardTarget(region)) return
  const previous = lastListControl
  const target =
    previous && isVisibleKeyboardTarget(previous) && !previous.matches(':disabled')
      ? previous
      : region.querySelector<HTMLElement>('[data-settings-list-heading]') || region
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'nearest', inline: 'nearest' })
}

function focusDetail() {
  const region = detail.value
  if (!region || !isVisibleKeyboardTarget(region)) return
  const heading = props.detailLoading ? null : region.querySelector<HTMLElement>('[data-settings-detail-heading]')
  const target = heading || region
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'nearest', inline: 'nearest' })
}

function focusStackedPane(open: boolean) {
  if (!props.backLabel) return
  const hiddenPane = open ? list.value : detail.value
  if (!hiddenPane || isVisibleKeyboardTarget(hiddenPane)) return
  if (open) focusDetail()
  else focusList()
}

// A direct link can open an already-selected detail before this component mounts.
onMounted(() => {
  if (detailOpen.value) focusStackedPane(true)
})
watch(detailOpen, focusStackedPane, { flush: 'post' })

watch(
  () => props.detailLoading,
  (loading) => {
    if (!loading && document.activeElement === detail.value) focusDetail()
  },
  { flush: 'post' },
)

defineExpose({ focusDetail, focusList, scrollListToTop: () => listBody.value?.scrollTo({ top: 0 }) })
</script>

<template>
  <div class="settings-master-detail settings-viewport-region">
    <aside
      ref="list"
      class="flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border border-border/70 bg-card/30 outline-none"
      :class="{ 'settings-hide-stacked': backLabel && detailOpen && hasDetail, 'col-span-full': !hasDetail }"
      :aria-label="listLabel"
      tabindex="-1"
      @focusin="rememberListControl"
    >
      <div v-if="$slots['list-header']" class="shrink-0 space-y-2 border-b border-border/60 bg-muted/20 p-2.5">
        <slot name="list-header" />
      </div>
      <div
        ref="listBody"
        v-keyboard-collection
        :class="cn('settings-viewport-scroll space-y-1 p-1.5', listScrollClass)"
        :aria-busy="listBusy || undefined"
      >
        <slot name="list" />
      </div>
      <slot name="list-footer" />
    </aside>
    <section
      v-if="hasDetail"
      ref="detail"
      data-settings-detail
      :class="
        cn(
          'flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border border-border/70 bg-card/30 outline-none',
          backLabel && !detailOpen && 'settings-hide-stacked',
          detailClass,
        )
      "
      :aria-label="detailLabel"
      :aria-busy="detailLoading || undefined"
      tabindex="-1"
    >
      <div v-if="backLabel" class="settings-stacked-only shrink-0 border-b border-border/60 px-2 py-1">
        <Button size="sm" variant="ghost" @click="detailOpen = false">
          <RiArrowLeftLine class="me-1.5 size-4 rtl:rotate-180" aria-hidden="true" />{{ backLabel }}
        </Button>
      </div>
      <slot name="detail" />
    </section>
  </div>
</template>
