<script setup lang="ts">
import { type HTMLAttributes, computed } from 'vue'
import { RiArrowDownSLine, RiArrowRightSLine } from '@remixicon/vue'

import { cn } from '@/lib/utils'

type DisclosureTone = 'default' | 'dirty' | 'saving' | 'error' | 'disabled'

const props = withDefaults(
  defineProps<{
    open: boolean
    label: string
    summary?: string
    tone?: DisclosureTone
    nested?: boolean
    disabled?: boolean
    class?: HTMLAttributes['class']
    bodyClass?: HTMLAttributes['class']
  }>(),
  {
    summary: '',
    tone: 'default',
    nested: false,
    disabled: false,
    class: '',
    bodyClass: '',
  },
)

const emit = defineEmits<{
  (event: 'toggle'): void
}>()

const toneClass = computed(() => {
  if (props.tone === 'dirty') return 'border-amber-500/40 bg-amber-500/[0.04]'
  if (props.tone === 'saving') return 'border-primary/40 bg-primary/[0.04]'
  if (props.tone === 'error') return 'border-destructive/45 bg-destructive/[0.04]'
  if (props.tone === 'disabled') return 'opacity-60'
  return 'border-border/60 bg-background/35'
})
</script>

<template>
  <section :class="cn('overflow-hidden rounded-lg border', toneClass, props.class)">
    <div class="flex min-h-11 min-w-0 items-center gap-1 px-2 py-1.5">
      <button
        type="button"
        class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1 text-left outline-none hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-ring/60"
        :aria-expanded="open"
        :disabled="disabled"
        @click="emit('toggle')"
      >
        <RiArrowDownSLine v-if="open" class="h-4 w-4 shrink-0 text-muted-foreground" />
        <RiArrowRightSLine v-else class="h-4 w-4 shrink-0 text-muted-foreground" />
        <slot name="leading" />
        <span class="min-w-0 flex-1">
          <span class="flex min-w-0 items-center gap-2">
            <span class="truncate text-sm font-semibold">{{ label }}</span>
            <slot name="badges" />
          </span>
          <span v-if="summary" class="mt-0.5 block truncate text-[11px] text-muted-foreground">{{ summary }}</span>
        </span>
      </button>

      <div v-if="$slots.actions" class="flex shrink-0 items-center gap-0.5" @click.stop>
        <slot name="actions" />
      </div>
    </div>

    <div v-if="open" :class="cn('border-t border-border/55', nested ? 'bg-muted/[0.04] p-2' : 'p-3 lg:p-4', bodyClass)">
      <slot />
    </div>
  </section>
</template>
