<script setup lang="ts">
import { nextTick, ref } from 'vue'
import { RiArrowDownSLine, RiArrowUpSLine, RiCloseLine, RiLock2Line } from '@remixicon/vue'
import IconButton from '@/components/ui/IconButton.vue'
import { focusKeyboardItem, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

const props = defineProps<{
  rowId: string
  title: string
  excerpt?: string
  position?: number
  compact?: boolean
  selection?: 'pressed' | 'expanded'
  selected?: boolean
  reorderable?: boolean
  removable?: boolean
  canMoveUp?: boolean
  canMoveDown?: boolean
  disabled?: boolean
  locked?: boolean
}>()
const emit = defineEmits<{ select: []; move: [delta: -1 | 1]; remove: [] }>()
const primary = ref<HTMLElement | null>(null)

async function move(event: MouseEvent, delta: -1 | 1) {
  const control = event.currentTarget
  const hadFocus = control === document.activeElement
  emit('move', delta)
  await nextTick()
  // Moving a keyed DOM row can blur its button; at an edge that button can
  // also become disabled. Keep navigation on the row that just moved.
  if (
    hadFocus &&
    (document.activeElement === document.body || document.activeElement === control) &&
    primary.value &&
    isVisibleKeyboardTarget(primary.value)
  ) {
    focusKeyboardItem(primary.value)
  }
}
</script>

<template>
  <li
    :data-keyboard-item="rowId"
    :data-keyboard-label="title"
    :data-keyboard-selected="selection ? Boolean(selected) : undefined"
    class="flex min-w-0 items-center rounded-md pe-1 hover:bg-muted/45"
    :class="selected && 'bg-primary/10 ring-1 ring-inset ring-primary/25'"
  >
    <component
      :is="selection ? 'button' : 'div'"
      ref="primary"
      :type="selection ? 'button' : undefined"
      :tabindex="selection ? undefined : 0"
      :aria-pressed="selection === 'pressed' ? Boolean(selected) : undefined"
      :aria-expanded="selection === 'expanded' ? Boolean(selected) : undefined"
      data-keyboard-primary
      class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-1.5 text-start outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      :class="compact ? 'text-xs' : 'text-sm'"
      @click="props.selection && emit('select')"
    >
      <span
        v-if="position !== undefined"
        aria-hidden="true"
        class="w-6 shrink-0 text-center font-mono text-xs tabular-nums text-muted-foreground"
      >
        {{ position }}
      </span>
      <span class="min-w-0 flex-1">
        <span class="block truncate font-medium" :title="title">{{ title }}</span>
        <span v-if="excerpt" class="block truncate text-xs text-muted-foreground">{{ excerpt }}</span>
      </span>
    </component>
    <div v-if="reorderable || removable || locked" class="flex shrink-0 items-center">
      <template v-if="reorderable">
        <IconButton
          size="sm"
          :aria-label="$st('Move up')"
          :tooltip="$st('Move up')"
          :disabled="disabled || !canMoveUp"
          @click="move($event, -1)"
        >
          <RiArrowUpSLine class="size-4" aria-hidden="true" />
        </IconButton>
        <IconButton
          size="sm"
          :aria-label="$st('Move down')"
          :tooltip="$st('Move down')"
          :disabled="disabled || !canMoveDown"
          @click="move($event, 1)"
        >
          <RiArrowDownSLine class="size-4" aria-hidden="true" />
        </IconButton>
      </template>
      <span v-if="locked" class="grid size-7 place-items-center text-primary" :title="$st('Required built-in prompt')">
        <RiLock2Line class="size-4" role="img" :aria-label="$st('Required built-in prompt')" />
      </span>
      <IconButton
        v-else-if="removable"
        size="sm"
        :aria-label="$st('Remove')"
        :tooltip="$st('Remove')"
        :disabled="disabled"
        @click="emit('remove')"
      >
        <RiCloseLine class="size-4" aria-hidden="true" />
      </IconButton>
    </div>
  </li>
</template>
