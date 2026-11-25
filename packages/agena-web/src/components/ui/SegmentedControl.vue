<script setup lang="ts">
import { type HTMLAttributes } from 'vue'

import { handleTabKeydown } from '@/lib/keyboardNavigation'
import { cn } from '@/lib/utils'

interface Props {
  class?: HTMLAttributes['class']
  role?: 'tablist' | 'group'
  variant?: 'segmented' | 'underline'
}

const props = withDefaults(defineProps<Props>(), { role: 'tablist', variant: 'segmented' })
</script>

<template>
  <div
    :role="role"
    :class="
      cn(
        variant === 'underline'
          ? 'flex min-w-0 shrink-0 gap-1 overflow-x-auto border-b border-border/60'
          : 'grid gap-1 rounded-sm border border-sidebar-border/70 bg-sidebar-accent/20 p-0.5',
        props.class,
      )
    "
    @keydown="handleTabKeydown($event, role === 'tablist')"
  >
    <slot />
  </div>
</template>
