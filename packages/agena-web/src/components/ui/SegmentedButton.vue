<script setup lang="ts">
import { type HTMLAttributes, computed } from 'vue'

import { cn } from '@/lib/utils'

type SegmentedButtonSize = 'sm' | 'xs'

interface Props {
  active?: boolean
  size?: SegmentedButtonSize
  role?: 'tab' | 'button'
  variant?: 'segmented' | 'underline'
  class?: HTMLAttributes['class']
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  size: 'xs',
  role: 'tab',
  variant: 'segmented',
})

const buttonClass = computed(() => {
  const inactive = 'text-muted-foreground hover:text-foreground'
  if (props.variant === 'underline') {
    return cn(
      'flex h-10 shrink-0 items-center gap-1.5 whitespace-nowrap border-b-2 px-3 transition-colors',
      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
      props.size === 'sm' ? 'text-sm' : 'text-xs',
      props.active ? 'border-primary font-medium text-foreground' : cn('border-transparent', inactive),
    )
  }
  return cn(
    props.size === 'sm' ? 'h-7 rounded-sm px-2.5 text-xs' : 'h-6 rounded-[5px] px-2 text-[11px]',
    'font-medium transition focus-visible:outline-2 focus-visible:outline-ring',
    props.active ? 'bg-sidebar-accent/80 text-foreground' : inactive,
  )
})
</script>

<template>
  <button
    type="button"
    :role="role"
    :aria-selected="role === 'tab' ? active : undefined"
    :tabindex="role === 'tab' ? (active ? 0 : -1) : undefined"
    data-keyboard-option
    :class="cn(buttonClass, props.class)"
  >
    <slot />
  </button>
</template>
