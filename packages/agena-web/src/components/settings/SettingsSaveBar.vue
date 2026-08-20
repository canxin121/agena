<script setup lang="ts">
import { RiCheckboxCircleLine, RiErrorWarningLine, RiLoader4Line, RiResetLeftLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'

withDefaults(
  defineProps<{
    dirty: boolean
    saving?: boolean
    disabled?: boolean
    error?: string
    saveLabel?: string
    sticky?: boolean
  }>(),
  {
    saving: false,
    disabled: false,
    error: '',
    saveLabel: '',
    sticky: false,
  },
)

const emit = defineEmits<{
  (event: 'save'): void
  (event: 'discard'): void
}>()
</script>

<template>
  <div
    class="flex flex-wrap items-center justify-between gap-3 rounded-lg border px-3 py-2.5 shadow-sm backdrop-blur"
    :class="[
      dirty ? 'border-amber-500/35 bg-amber-500/[0.06]' : 'border-border/60 bg-background/90',
      sticky ? 'sticky bottom-3 z-20' : '',
    ]"
  >
    <div class="flex min-w-0 items-center gap-2 text-xs">
      <RiLoader4Line v-if="saving" class="h-4 w-4 shrink-0 animate-spin text-primary" />
      <RiErrorWarningLine v-else-if="error" class="h-4 w-4 shrink-0 text-destructive" />
      <span v-else-if="dirty" class="h-2 w-2 shrink-0 rounded-full bg-amber-500" />
      <RiCheckboxCircleLine v-else class="h-4 w-4 shrink-0 text-emerald-500" />

      <span v-if="saving" class="font-medium">{{ $st('Saving changes…') }}</span>
      <span v-else-if="error" class="min-w-0 break-words text-destructive">{{ error }}</span>
      <span v-else-if="dirty" class="font-medium text-amber-800 dark:text-amber-200">{{
        $st('You have unsaved changes.')
      }}</span>
      <span v-else class="text-muted-foreground">{{ $st('All changes are saved.') }}</span>
    </div>

    <div v-if="dirty || saving" class="flex shrink-0 items-center gap-2">
      <Button variant="ghost" size="sm" :disabled="saving || disabled" @click="emit('discard')">
        <RiResetLeftLine class="mr-1.5 h-4 w-4" />
        {{ $st('Discard changes') }}
      </Button>
      <Button size="sm" :disabled="saving || disabled || !dirty" @click="emit('save')">
        <RiSave3Line class="mr-1.5 h-4 w-4" />
        {{ saveLabel || $st('Save changes') }}
      </Button>
    </div>
  </div>
</template>
