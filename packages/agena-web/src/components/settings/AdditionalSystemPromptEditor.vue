<script setup lang="ts">
import { computed, useId } from 'vue'

import OptionPicker from '@/components/ui/OptionPicker.vue'
import {
  additionalSystemPromptIsValid,
  type AdditionalSystemPromptDraft,
  type AdditionalSystemPromptMode,
} from '@/lib/additionalSystemPrompt'
import { settingsText as st } from '@/i18n/settingsText'

const props = defineProps<{
  modelValue: AdditionalSystemPromptDraft
  disabled?: boolean
}>()
const emit = defineEmits<{
  (event: 'update:modelValue', value: AdditionalSystemPromptDraft): void
}>()
const inputId = useId()
const options = computed(() => [
  { value: 'inherit', label: st('Inherit'), description: st('Use the next available scope.') },
  { value: 'custom', label: st('Custom text'), description: st('Use these instructions at this scope.') },
  { value: 'disabled', label: st('Disabled'), description: st('Disable inherited additional instructions.') },
])
const valid = computed(() => additionalSystemPromptIsValid(props.modelValue))

function setMode(mode: string) {
  if (mode !== 'inherit' && mode !== 'custom' && mode !== 'disabled') return
  emit('update:modelValue', { ...props.modelValue, mode: mode as AdditionalSystemPromptMode })
}
</script>

<template>
  <div class="grid min-w-0 gap-3">
    <OptionPicker
      :model-value="modelValue.mode"
      :options="options"
      :include-empty="false"
      :title="$st('Mode')"
      :disabled="disabled"
      @update:model-value="setMode"
    />
    <p class="text-xs leading-relaxed text-muted-foreground">
      {{
        $st(
          'Built-in instructions always remain active. Disabled saves an empty override for additional instructions only.',
        )
      }}
    </p>
    <label v-if="modelValue.mode === 'custom'" :for="inputId" class="grid min-w-0 gap-1.5">
      <span class="text-xs text-muted-foreground">{{ $st('Additional instructions') }}</span>
      <textarea
        :id="inputId"
        :value="modelValue.text"
        :disabled="disabled"
        :aria-invalid="!valid || undefined"
        :aria-describedby="!valid ? inputId + '-error' : undefined"
        rows="9"
        spellcheck="false"
        class="w-full resize-y rounded-md border border-input bg-transparent px-3 py-2 text-sm leading-relaxed outline-none focus:border-ring"
        @input="emit('update:modelValue', { ...modelValue, text: ($event.target as HTMLTextAreaElement).value })"
      />
      <span v-if="!valid" :id="inputId + '-error'" class="text-xs text-destructive" role="status">
        {{ $st('Enter instructions or choose Disabled.') }}
      </span>
    </label>
    <p v-else class="text-xs text-muted-foreground">
      {{
        modelValue.mode === 'inherit'
          ? $st('Use the next available scope.')
          : $st('Disable inherited additional instructions.')
      }}
    </p>
  </div>
</template>
