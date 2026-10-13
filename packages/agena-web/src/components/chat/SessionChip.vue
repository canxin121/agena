<script setup lang="ts">
// One compact composer-footer chip per session section. The chip shows an icon
// and a short label; its panel lives in a portal so neither the composer nor the
// split pane can clip it. `data-session-section` stays on the panel because the
// shared markdown typography in style.css keys off it.
import { RiLoader4Line } from '@remixicon/vue'
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from 'radix-vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  title: string
  label?: string
  hint?: string
  open: boolean
  busy?: boolean
}>()
defineEmits<{ (event: 'update:open', value: boolean): void }>()
</script>

<template>
  <PopoverRoot :open="open" @update:open="$emit('update:open', $event)">
    <PopoverTrigger as-child>
      <button
        v-bind="$attrs"
        type="button"
        data-oc-keyboard-tap="blur"
        data-session-chip
        class="pointer-events-auto flex min-h-7 shrink-0 touch-manipulation items-center gap-1.5 rounded-md border border-border/60 bg-background px-1.5 py-0.5 text-muted-foreground hover:bg-secondary/50 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        :class="open ? 'border-ring/50 bg-secondary/60 text-foreground' : ''"
        :title="hint || title"
        :aria-label="hint || title"
        :aria-expanded="open"
      >
        <slot name="icon" />
        <span v-if="label" class="max-w-32 truncate">{{ label }}</span>
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
        data-transcript-chrome="true"
        data-session-section
      >
        <div class="mb-1.5 flex items-center gap-2">
          <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ title }}</span>
          <slot name="actions" />
        </div>
        <div class="max-h-[min(70dvh,40rem)] min-w-0 overflow-auto overscroll-contain">
          <slot />
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
