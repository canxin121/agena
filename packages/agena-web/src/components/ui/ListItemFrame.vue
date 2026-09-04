<script setup lang="ts">
import { computed, useSlots } from 'vue'
import { cn } from '@/lib/utils'

type ListItemActionVisibility = 'hover' | 'always'
type ListItemDensity = 'default' | 'compact'

const props = withDefaults(
  defineProps<{
    active?: boolean
    indent?: number
    disabled?: boolean
    as?: string
    actionVisibility?: ListItemActionVisibility
    density?: ListItemDensity
    actionsFloating?: boolean
    iconClass?: string
    contentClass?: string
    metaClass?: string
  }>(),
  {
    active: false,
    indent: undefined,
    disabled: false,
    as: 'button',
    actionVisibility: 'hover',
    density: 'default',
    actionsFloating: true,
    iconClass: '',
    contentClass: '',
    metaClass: '',
  },
)

const emit = defineEmits<{
  (e: 'click', event: MouseEvent): void
}>()

const slots = useSlots()

// Rows frequently expose secondary actions (more, delete, favorite, etc.).
// Rendering those actions inside the default <button> root creates nested
// buttons, which is invalid HTML and produces inconsistent pointer/focus
// behavior across browsers. When actions exist, use a neutral flex root and
// keep the primary row action as a real button alongside the action buttons.
const usesButtonSurrogate = computed(() => props.as === 'button' && Boolean(slots.actions))
const resolvedAs = computed(() => (usesButtonSurrogate.value ? 'div' : props.as))

const rootClass = computed(() => {
  const densityClass = props.density === 'compact' ? 'py-0.5 pl-2 pr-1.5' : 'py-1 pl-2 pr-1.5'
  return cn(
    'group flex w-full min-w-0 items-center gap-2 rounded-md text-left text-sm transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
    densityClass,
    props.active
      ? 'bg-primary/12 text-foreground dark:bg-accent/80 font-medium'
      : 'text-muted-foreground hover:bg-primary/6 hover:text-foreground hover:dark:bg-accent/40',
    props.disabled && 'pointer-events-none opacity-50',
  )
})

const actionsClass = computed(() => {
  const visibilityClass =
    props.actionVisibility === 'always'
      ? 'max-w-full opacity-100 pointer-events-auto'
      : 'max-w-0 opacity-0 pointer-events-none group-hover:max-w-full group-hover:opacity-100 group-hover:pointer-events-auto group-focus-within:max-w-full group-focus-within:opacity-100 group-focus-within:pointer-events-auto'
  const floatingClass = props.actionsFloating ? 'z-[1]' : ''
  return cn(
    'ml-1 flex min-w-0 shrink-0 items-center gap-0.5 overflow-hidden transition-[max-width,opacity] duration-200 ease-out',
    visibilityClass,
    floatingClass,
  )
})

function handleRootClick(event: MouseEvent) {
  // The split primary button stops propagation. In split mode this fallback
  // keeps non-interactive leading affordances (icons/selection indicators)
  // clickable as part of the row without wrapping real controls in a button.
  emit('click', event)
}
</script>

<template>
  <component
    :is="resolvedAs"
    :type="resolvedAs === 'button' ? 'button' : undefined"
    data-oc-list-item-frame
    data-oc-actions-lock-frame
    :class="rootClass"
    :style="{ paddingLeft: typeof indent === 'number' ? `${indent}px` : undefined }"
    :disabled="resolvedAs === 'button' ? disabled : undefined"
    @click="handleRootClick"
  >
    <div
      v-if="$slots.leading"
      :class="cn('flex shrink-0 items-center justify-center text-muted-foreground/70', iconClass)"
    >
      <slot name="leading" />
    </div>

    <button
      v-if="usesButtonSurrogate"
      type="button"
      data-oc-list-item-primary
      class="flex min-w-0 flex-1 items-center gap-2 self-stretch bg-transparent p-0 text-left text-inherit outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      :disabled="disabled"
      @click.stop="emit('click', $event)"
    >
      <div :class="cn('flex min-w-0 flex-1 flex-col justify-center overflow-visible', contentClass)">
        <slot />
      </div>

      <div
        v-if="$slots.meta"
        :class="cn('flex shrink-0 items-center gap-1 text-[10px] text-muted-foreground', metaClass)"
      >
        <slot name="meta" />
      </div>
    </button>

    <template v-else>
      <div :class="cn('flex min-w-0 flex-1 flex-col justify-center overflow-visible', contentClass)">
        <slot />
      </div>

      <div
        v-if="$slots.meta"
        :class="cn('flex shrink-0 items-center gap-1 text-[10px] text-muted-foreground', metaClass)"
      >
        <slot name="meta" />
      </div>
    </template>

    <div v-if="$slots.actions" :class="cn('oc-list-item-actions', actionsClass)" @click.stop>
      <slot name="actions" />
    </div>
  </component>
</template>

<style scoped>
[data-oc-actions-lock-frame][data-oc-actions-locked='true'] .oc-list-item-actions {
  max-width: 100% !important;
  opacity: 1 !important;
  pointer-events: auto !important;
}
</style>
