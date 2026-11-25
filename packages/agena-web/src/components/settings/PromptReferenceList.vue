<script setup lang="ts">
import { computed, ref } from 'vue'
import { RiAddLine } from '@remixicon/vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { vKeyboardCollection } from '@/lib/keyboardCollection'
import { savedPromptDisplayName, savedPromptExcerpt } from '@/lib/promptLibrary'
import type { PromptFragment } from '@/lib/promptLayers'
import { settingsText as st } from '@/i18n/settingsText'
import PromptOrderRow from './PromptOrderRow.vue'

const props = defineProps<{
  modelValue: string[]
  options: PromptFragment[]
  disabled?: boolean
  selectLabel?: string
  limit?: number
}>()
const emit = defineEmits<{ 'update:modelValue': [ids: string[]] }>()
const list = ref<HTMLElement | null>(null)
const byId = computed(() => new Map(props.options.map((prompt) => [prompt.id, prompt])))
const rows = computed(() =>
  props.modelValue.map((id) => {
    const prompt = byId.value.get(id)
    return {
      id,
      title: prompt ? savedPromptDisplayName(prompt) : id,
      excerpt: prompt ? savedPromptExcerpt(prompt, byId.value) : '',
    }
  }),
)
const choices = computed(() => {
  const used = new Set(props.modelValue)
  return props.options
    .filter((prompt) => !used.has(prompt.id))
    .map((prompt) => ({
      value: prompt.id,
      label: savedPromptDisplayName(prompt),
      description: savedPromptExcerpt(prompt, byId.value),
      group: prompt.category || st('Uncategorized'),
    }))
    .sort((a, b) => a.group.localeCompare(b.group))
})
const pickerLabel = computed(() => props.selectLabel || st('Select a saved prompt'))
const addDisabled = computed(
  () => props.disabled || !choices.value.length || (props.limit != null && props.modelValue.length >= props.limit),
)

function add(id: string) {
  if (addDisabled.value || !choices.value.some((choice) => choice.value === id)) return
  emit('update:modelValue', [...props.modelValue, id])
}

function returnFocusTarget() {
  // Selecting the final option (or reaching the limit) disables the trigger.
  return (
    list.value?.querySelector<HTMLElement>('[data-prompt-reference-add] button:not(:disabled)') ??
    list.value?.querySelector<HTMLElement>('[data-keyboard-primary][tabindex="0"]') ??
    list.value
  )
}

function move(index: number, offset: number) {
  const next = [...props.modelValue]
  const other = index + offset
  if (props.disabled || other < 0 || other >= next.length) return
  const [id] = next.splice(index, 1)
  if (!id) return
  next.splice(other, 0, id)
  emit('update:modelValue', next)
}

function remove(id: string) {
  if (!props.disabled)
    emit(
      'update:modelValue',
      props.modelValue.filter((entry) => entry !== id),
    )
}
</script>

<template>
  <div
    ref="list"
    v-keyboard-collection
    class="grid min-w-0 grid-cols-1 gap-1.5 rounded-md focus-visible:outline-2 focus-visible:outline-ring"
  >
    <ol v-if="rows.length" class="divide-y divide-border/60 rounded-md border border-border/60">
      <PromptOrderRow
        v-for="({ id, title, excerpt }, index) in rows"
        :key="id"
        :row-id="id"
        :title="title"
        :excerpt="excerpt"
        :can-move-up="index > 0"
        :can-move-down="index + 1 < rows.length"
        :disabled="disabled"
        compact
        reorderable
        removable
        @move="move(index, $event)"
        @remove="remove(id)"
      />
    </ol>
    <OptionPicker
      model-value=""
      :options="choices"
      :title="pickerLabel"
      :placeholder="pickerLabel"
      :include-empty="false"
      :disabled="addDisabled"
      :icon="RiAddLine"
      :return-focus-target="returnFocusTarget"
      size="sm"
      data-prompt-reference-add
      @update:model-value="add"
    />
  </div>
</template>
