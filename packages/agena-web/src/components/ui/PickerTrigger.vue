<script setup lang="ts">
import { ref, type Component, type HTMLAttributes } from 'vue'
import { optionOpenEdge } from '@/lib/keyboardNavigation'
import { cn } from '@/lib/utils'

const props = defineProps<{
  open: boolean
  label: string
  ariaLabel?: string
  icon?: Component
  disabled?: boolean
  muted?: boolean
  block?: boolean
  size?: 'default' | 'sm'
  monospace?: boolean
  class?: HTMLAttributes['class']
}>()
const emit = defineEmits<{ toggle: []; 'open-at': [edge: 'first' | 'last'] }>()
const triggerEl = ref<HTMLButtonElement | null>(null)
defineExpose({ triggerEl })

function onKeydown(event: KeyboardEvent) {
  const edge = optionOpenEdge(event)
  if (!edge || props.open || props.disabled) return
  event.preventDefault()
  event.stopPropagation()
  emit('open-at', edge)
}
</script>

<template>
  <button
    ref="triggerEl"
    type="button"
    :disabled="disabled"
    :aria-label="ariaLabel || label"
    aria-haspopup="dialog"
    :aria-expanded="open"
    :class="
      cn(
        'inline-flex min-w-0 items-center justify-between gap-2 rounded-md border border-input bg-transparent text-start shadow-sm transition-colors hover:bg-secondary/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50',
        block ? 'w-full' : 'max-w-full',
        size === 'sm' ? 'h-8 px-2 text-xs' : 'h-9 px-3 text-sm',
        props.class,
      )
    "
    @click.stop="emit('toggle')"
    @keydown="onKeydown"
  >
    <span class="flex min-w-0 items-center gap-2">
      <component v-if="icon" :is="icon" class="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="truncate font-medium" :class="[muted && 'text-muted-foreground', monospace && 'font-mono']">
        {{ label }}
      </span>
    </span>
    <slot />
  </button>
</template>
