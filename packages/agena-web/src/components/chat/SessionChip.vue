<script setup lang="ts">
// One compact composer-footer chip per session section. The chip shows an icon
// and a short label; its panel lives in a portal so neither the composer nor the
// split pane can clip it. `data-session-section` stays on the panel because the
// shared markdown typography in style.css keys off it.
import { RiLoader4Line } from '@remixicon/vue'
import { ref, watch } from 'vue'
import { PopoverTrigger } from 'radix-vue'
import PopoverSurface from '@/components/ui/PopoverSurface.vue'
import {
  enabledKeyboardItems,
  focusKeyboardItem,
  handleOptionKeydown,
  isPlainNavigationKey,
} from '@/lib/keyboardNavigation'
import { cn } from '@/lib/utils'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  title: string
  label?: string
  hint?: string
  open: boolean
  busy?: boolean
  panelClass?: string
}>()
defineEmits<{ (event: 'update:open', value: boolean): void }>()
const bodyEl = ref<HTMLElement | null>(null)
let lastFocused: HTMLElement | null = null

function onPanelFocusin(event: FocusEvent) {
  if (event.target instanceof HTMLElement) lastFocused = event.target
}

watch(
  bodyEl,
  (body, _previous, onCleanup) => {
    const panel = body?.closest('[data-session-section]')
    if (!body || !panel) return
    const observer = new MutationObserver(() => {
      if (!props.open || !lastFocused || lastFocused.isConnected || document.activeElement !== document.body) return
      const [first] = enabledKeyboardItems(body)
      if (!focusKeyboardItem(first)) body.focus({ preventScroll: true })
    })
    observer.observe(panel, { childList: true, subtree: true })
    onCleanup(() => {
      observer.disconnect()
      lastFocused = null
    })
  },
  { flush: 'post' },
)

function scrollBody(event: KeyboardEvent) {
  const body = bodyEl.value
  if (handleOptionKeydown(event, body)) return
  if (!body || event.target !== body || !isPlainNavigationKey(event)) return
  const page = Math.max(40, body.clientHeight * 0.9)
  const distance: Record<string, number> = { ArrowUp: -40, ArrowDown: 40, PageUp: -page, PageDown: page }
  if (event.key === ' ' || (!event.shiftKey && event.key in distance)) {
    event.preventDefault()
    event.stopPropagation()
    body.scrollBy({
      top: event.key === ' ' ? (event.shiftKey ? -page : page) : distance[event.key],
      behavior: 'instant',
    })
  } else if (!event.shiftKey && (event.key === 'Home' || event.key === 'End')) {
    event.preventDefault()
    event.stopPropagation()
    body.scrollTo({ top: event.key === 'Home' ? 0 : body.scrollHeight, behavior: 'instant' })
  }
}
</script>

<template>
  <PopoverSurface
    :open="open"
    side="top"
    align="start"
    :side-offset="8"
    :collision-padding="10"
    sticky="always"
    :class="cn('flex w-[min(92vw,42rem)] flex-col overflow-hidden px-3 py-2', panelClass)"
    :aria-label="title"
    data-transcript-chrome="true"
    data-session-section
    initial-focus="[data-session-section-body]"
    @update:open="$emit('update:open', $event)"
    @focusin="onPanelFocusin"
  >
    <template #trigger>
      <PopoverTrigger as-child>
        <button
          v-bind="$attrs"
          type="button"
          data-oc-keyboard-tap="blur"
          data-session-chip
          class="pointer-events-auto flex min-h-7 shrink-0 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :class="open ? 'bg-secondary/60 text-foreground' : ''"
          :title="hint || title"
          :aria-label="hint || title"
        >
          <slot name="trigger">
            <slot name="icon" />
            <span v-if="label" class="max-w-32 truncate font-mono text-[11px] font-medium">{{ label }}</span>
            <RiLoader4Line v-if="busy" class="h-3 w-3 shrink-0 animate-spin" />
          </slot>
        </button>
      </PopoverTrigger>
    </template>
    <div class="mb-1.5 flex shrink-0 items-center gap-2">
      <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ title }}</span>
      <slot name="actions" />
    </div>
    <div
      ref="bodyEl"
      tabindex="0"
      role="region"
      :aria-label="title"
      data-session-section-body
      class="max-h-[min(70dvh,40rem)] min-h-0 min-w-0 overflow-auto overscroll-contain rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      @keydown="scrollBody"
    >
      <slot />
    </div>
  </PopoverSurface>
</template>
