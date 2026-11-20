<script setup lang="ts">
import { RiCloseLine, RiDeleteBinLine } from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import type { PickerOption } from '@/components/ui/pickerOption.types'

defineProps<{
  count: number
  options: PickerOption[]
  disabled?: boolean
  modeDisabled?: boolean
  clearDisabled?: boolean
}>()
const mode = defineModel<string>({ required: true })
const emit = defineEmits<{ apply: []; clear: []; remove: [] }>()
</script>

<template>
  <div class="permission-bulk-actions">
    <div class="permission-bulk-selection">
      <span role="status" class="text-[13px] font-semibold tabular-nums">
        {{ $st('{count} selected', { count }) }}
      </span>
      <IconButton
        variant="ghost"
        size="md"
        class="text-muted-foreground pointer-coarse:w-9"
        :aria-label="$st('Select none')"
        :tooltip="$st('Select none')"
        :disabled="clearDisabled"
        @click="emit('clear')"
      >
        <RiCloseLine class="h-4 w-4" aria-hidden="true" />
      </IconButton>
    </div>
    <div class="permission-bulk-controls">
      <div class="permission-bulk-mode">
        <OptionPicker
          v-model="mode"
          :options="options"
          :include-empty="false"
          :title="$st('Set mode for selected rules')"
          :disabled="disabled || modeDisabled"
          trigger-class="h-full min-h-8 rounded-e-none bg-background py-1 text-start shadow-none"
          size="sm"
        />
        <Button
          variant="outline"
          size="sm"
          class="h-full min-h-8 rounded-s-none border-s-0 shadow-none"
          :disabled="disabled || modeDisabled"
          @click="emit('apply')"
        >
          {{ $st('Apply') }}
        </Button>
      </div>
      <slot name="remove">
        <IconButton
          variant="ghost-destructive"
          size="md"
          class="pointer-coarse:w-9"
          :aria-label="$st('Remove selected')"
          :tooltip="$st('Remove selected')"
          :disabled="disabled"
          @click="emit('remove')"
        >
          <RiDeleteBinLine class="h-4 w-4" aria-hidden="true" />
        </IconButton>
      </slot>
    </div>
  </div>
</template>

<style scoped>
@reference '../../../style.css';

.permission-bulk-actions {
  @apply rounded-lg border border-primary/20 bg-primary/5;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem 0.75rem;
  min-width: 0;
  padding: 0.25rem 0.5rem;
}
.permission-bulk-selection,
.permission-bulk-controls {
  display: flex;
  align-items: center;
  min-width: 0;
}
.permission-bulk-selection {
  gap: 0.25rem;
  overflow-wrap: anywhere;
}
.permission-bulk-controls {
  flex: 1 1 12rem;
  justify-content: flex-end;
  gap: 0.375rem;
}
.permission-bulk-mode {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  width: min(100%, 12rem);
  min-width: 0;
}
.permission-bulk-mode :deep(.truncate) {
  overflow: visible;
  overflow-wrap: anywhere;
  text-overflow: clip;
  white-space: normal;
  line-height: 1.25;
}
@media (pointer: coarse) {
  .permission-bulk-actions :deep(button) {
    min-height: 2.375rem;
  }
}
</style>
