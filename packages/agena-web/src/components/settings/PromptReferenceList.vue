<script setup lang="ts">
import { computed } from 'vue'
import { RiArrowDownSLine, RiArrowUpSLine, RiCloseLine } from '@remixicon/vue'
import IconButton from '@/components/ui/IconButton.vue'
import { savedPromptDisplayName } from '@/lib/promptLibrary'
import type { PromptFragment } from '@/lib/promptLayers'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{ modelValue: string[]; options: PromptFragment[]; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [ids: string[]] }>()
const names = computed(() => new Map(props.options.map((prompt) => [prompt.id, savedPromptDisplayName(prompt)])))
const categories = computed(() => {
  const groups = new Map<string, PromptFragment[]>()
  for (const prompt of props.options) {
    if (props.modelValue.includes(prompt.id)) continue
    const category = prompt.category || st('Uncategorized')
    if (!groups.has(category)) groups.set(category, [])
    groups.get(category)!.push(prompt)
  }
  return [...groups.entries()].sort(([left], [right]) => left.localeCompare(right))
})

function add(event: Event) {
  const select = event.target as HTMLSelectElement
  if (select.value) emit('update:modelValue', [...props.modelValue, select.value])
  select.value = ''
}

function move(index: number, offset: number) {
  const next = [...props.modelValue]
  const other = index + offset
  if (other < 0 || other >= next.length) return
  const [id] = next.splice(index, 1)
  next.splice(other, 0, id!)
  emit('update:modelValue', next)
}
</script>

<template>
  <div class="grid min-w-0 gap-1.5">
    <ol v-if="modelValue.length" class="divide-y divide-border/60 rounded-md border border-border/60">
      <li v-for="(id, index) in modelValue" :key="id" class="flex items-center gap-2 ps-2 text-xs">
        <span class="min-w-0 flex-1 break-words py-1.5">{{ names.get(id) || id }}</span>
        <div class="flex shrink-0">
          <IconButton
            size="sm"
            :aria-label="$st('Move up')"
            :tooltip="$st('Move up')"
            :disabled="disabled || index === 0"
            @click="move(index, -1)"
          >
            <RiArrowUpSLine class="h-4 w-4" aria-hidden="true" />
          </IconButton>
          <IconButton
            size="sm"
            :aria-label="$st('Move down')"
            :tooltip="$st('Move down')"
            :disabled="disabled || index + 1 === modelValue.length"
            @click="move(index, 1)"
          >
            <RiArrowDownSLine class="h-4 w-4" aria-hidden="true" />
          </IconButton>
          <IconButton
            size="sm"
            :aria-label="$st('Remove')"
            :tooltip="$st('Remove')"
            :disabled="disabled"
            @click="
              emit(
                'update:modelValue',
                modelValue.filter((entry) => entry !== id),
              )
            "
          >
            <RiCloseLine class="h-4 w-4" aria-hidden="true" />
          </IconButton>
        </div>
      </li>
    </ol>
    <select
      class="h-8 min-w-0 rounded-md border border-input bg-background px-2 text-xs"
      :aria-label="$st('Select a saved prompt')"
      :disabled="disabled || !categories.length"
      @change="add"
    >
      <option value="">{{ $st('Select a saved prompt') }}</option>
      <optgroup v-for="[category, prompts] in categories" :key="category" :label="category">
        <option v-for="prompt in prompts" :key="prompt.id" :value="prompt.id">
          {{ savedPromptDisplayName(prompt) }}
        </option>
      </optgroup>
    </select>
  </div>
</template>
