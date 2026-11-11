<script setup lang="ts">
/**
 * A small chooser for an action that has exactly two outcomes.
 *
 * The app confirm dialog answers yes or no; this one asks which of two things
 * to do, so a destructive action that has two non-equivalent modes does not
 * have to hide one of them behind a second confirmation.
 */
import FormDialog from '@/components/ui/FormDialog.vue'
import Button from '@/components/ui/Button.vue'
import type { ChoiceDialogOption } from '@/components/ui/choiceDialog.types'

const props = withDefaults(
  defineProps<{
    open: boolean
    title: string
    description?: string
    options: ChoiceDialogOption[]
    cancelText?: string
  }>(),
  { description: '', cancelText: '' },
)

const emit = defineEmits<{
  (e: 'select', id: string): void
  (e: 'update:open', value: boolean): void
}>()

function choose(id: string) {
  emit('select', id)
  emit('update:open', false)
}
</script>

<template>
  <FormDialog
    :open="props.open"
    :title="props.title"
    :description="props.description"
    max-width="max-w-md"
    @update:open="(value) => emit('update:open', value)"
  >
    <div class="grid gap-2">
      <button
        v-for="option in props.options"
        :key="option.id"
        type="button"
        class="flex w-full flex-col gap-1 rounded-lg border border-border/60 px-3 py-2.5 text-left transition-colors hover:border-primary/50 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        @click="choose(option.id)"
      >
        <span class="text-sm font-medium">{{ option.label }}</span>
        <span v-if="option.description" class="text-xs text-muted-foreground">{{ option.description }}</span>
      </button>
    </div>
    <div class="mt-3 flex justify-end">
      <Button variant="ghost" size="sm" @click="emit('update:open', false)">
        {{ props.cancelText }}
      </Button>
    </div>
  </FormDialog>
</template>
