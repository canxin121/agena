<script setup lang="ts">
// One compact composer-footer chip per session section. The chip shows an icon
// and a short label; its panel lives in a portal so neither the composer nor the
// split pane can clip it. `data-session-section` stays on the panel because the
// shared markdown typography in style.css keys off it.
import { RiLoader4Line } from '@remixicon/vue'
import { ref } from 'vue'
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from 'radix-vue'
import { isPlainNavigationKey } from '@/lib/keyboardNavigation'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  title: string
  label?: string
  hint?: string
  open: boolean
  busy?: boolean
}>()
defineEmits<{ (event: 'update:open', value: boolean): void }>()
const bodyEl = ref<HTMLElement | null>(null)

function scrollBody(event: KeyboardEvent) {
  const body = bodyEl.value
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
  <PopoverRoot :open="open" @update:open="$emit('update:open', $event)">
    <PopoverTrigger as-child>
      <button
        tabindex="0"
        v-bind="$attrs"
        type="button"
        data-oc-keyboard-tap="blur"
        data-session-chip
        class="pointer-events-auto flex min-h-7 shrink-0 touch-manipulation items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="open ? 'bg-secondary/60 text-foreground' : ''"
        :title="hint || title"
        :aria-label="hint || title"
        :aria-expanded="open"
      >
        <slot name="icon" />
        <span v-if="label" class="max-w-32 truncate font-mono text-[11px] font-medium">{{ label }}</span>
        <RiLoader4Line v-if="busy" class="h-3 w-3 shrink-0 animate-spin" />
      </button>
    </PopoverTrigger>
    <PopoverPortal>
      <PopoverContent
        side="top"
        align="start"
        :side-offset="8"
        :avoid-collisions="true"
        :collision-padding="10"
        sticky="always"
        class="z-[80] w-[min(92vw,42rem)] rounded-xl border border-border bg-background px-3 py-2 shadow-xl outline-none"
        :aria-label="title"
        data-keyboard-overlay="true"
        data-transcript-chrome="true"
        data-session-section
        @open-auto-focus.prevent="bodyEl?.focus({ preventScroll: true })"
      >
        <div class="mb-1.5 flex items-center gap-2">
          <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ title }}</span>
          <slot name="actions" />
        </div>
        <div
          ref="bodyEl"
          tabindex="0"
          role="region"
          :aria-label="title"
          data-session-section-body
          class="max-h-[min(70dvh,40rem)] min-w-0 overflow-auto overscroll-contain rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          @keydown="scrollBody"
        >
          <slot />
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
