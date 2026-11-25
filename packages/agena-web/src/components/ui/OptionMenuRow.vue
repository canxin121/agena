<script setup lang="ts">
import { RiCheckLine } from '@remixicon/vue'
import ListRowButton from '@/components/ui/ListRowButton.vue'
import type { OptionMenuItem } from '@/components/ui/optionMenu.types'

defineProps<{ item: OptionMenuItem; compact: boolean }>()
</script>

<template>
  <ListRowButton
    data-keyboard-option
    data-option-menu-item
    :aria-current="item.checked || undefined"
    :active="item.checked"
    :size="compact ? 'md' : 'sm'"
    :class="compact ? 'min-h-11 items-center px-3 text-start' : 'items-start text-start text-xs'"
    :disabled="item.disabled"
    :destructive="item.variant === 'destructive'"
  >
    <component
      :is="item.icon"
      v-if="item.icon"
      aria-hidden="true"
      class="shrink-0 text-muted-foreground"
      :class="compact ? 'h-5 w-5' : 'mt-0.5 h-4 w-4'"
    />
    <div class="min-w-0 flex-1">
      <div class="truncate font-medium" :class="[compact ? 'text-sm' : 'text-xs', item.monospace && 'font-mono']">
        {{ item.label }}
      </div>
      <div v-if="item.description" class="truncate text-muted-foreground" :class="compact ? 'text-xs' : 'text-[11px]'">
        {{ item.description }}
      </div>
    </div>
    <RiCheckLine v-if="item.checked" class="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
  </ListRowButton>
</template>
