<script setup lang="ts">
import { computed } from 'vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { settingsText as st } from '@/i18n/settingsText'
import {
  speedModeOptionsForModel,
  thinkingModeOptionsForModel,
  verbosityOptionsForModel,
  type Provider,
} from '@/pages/chat/modelSelectionCatalog'
import { encodeModelSelectionKey, parseModelSlug } from '@/pages/chat/modelSelectionDefaults'
import type { JsonObject } from '@/types/json'

const props = defineProps<{
  modelValue: JsonObject
  providers: Provider[]
  disabled?: boolean
  requireRoute?: boolean
}>()
const emit = defineEmits<{ 'update:modelValue': [value: JsonObject] }>()

const key = computed(() => encodeModelSelectionKey(props.modelValue))
const identity = computed(() => parseModelSlug(key.value))
const routeOptions = computed(() => {
  const options = props.providers
    .flatMap((provider) =>
      provider.models.map((model) => {
        const adapter = String(model.adapter_id || '')
        return {
          value: encodeModelSelectionKey({ provider: provider.id, adapter, model: model.id }),
          label: String(model.display_name || model.id),
          description: [provider.id, adapter, model.id].filter(Boolean).join(' / '),
        }
      }),
    )
    .sort((left, right) => left.description.localeCompare(right.description))
  if (key.value && !options.some((option) => option.value === key.value)) {
    options.unshift({
      value: key.value,
      label: identity.value.model,
      description: st('Configured value'),
    })
  }
  return options
})
const partialRoute = computed(() =>
  ['provider', 'adapter', 'model']
    .map((part) => props.modelValue[part])
    .filter(Boolean)
    .join(' / '),
)
const selectedModel = computed(() => {
  const { provider, adapter, model } = identity.value
  return props.providers
    .find((entry) => entry.id === provider)
    ?.models.find((entry) => entry.id === model && (!adapter || adapter === entry.adapter_id))
})
const preferencesDisabled = computed(() => props.disabled || (props.requireRoute && !key.value))
const preferences = computed(() =>
  (
    [
      { key: 'thinking_mode', label: st('Thinking'), options: thinkingModeOptionsForModel(selectedModel.value) },
      { key: 'speed_mode', label: st('Speed'), options: speedModeOptionsForModel(selectedModel.value) },
      { key: 'verbosity', label: st('Verbosity'), options: verbosityOptionsForModel(selectedModel.value) },
    ] as const
  ).map((preference) => {
    const value = String(props.modelValue[preference.key] || '')
    const options = preference.options.map(({ value, label, description }) => ({ value, label, description }))
    if (value && !options.some((option) => option.value === value))
      options.push({ value, label: value, description: st('Configured value') })
    return { ...preference, value, options }
  }),
)
const parallelOptions = computed(() => [
  { value: 'true', label: st('Enabled') },
  { value: 'false', label: st('Disabled') },
])

function chooseRoute(value: string) {
  if (!value) return emit('update:modelValue', {})
  const { provider, adapter, model } = parseModelSlug(value)
  emit('update:modelValue', { provider, ...(adapter ? { adapter } : {}), model })
}
function setPreference(key: string, value: string) {
  const next = { ...props.modelValue }
  if (!value) delete next[key]
  else next[key] = key === 'parallel_tool_calls' ? value === 'true' : value
  emit('update:modelValue', next)
}
</script>

<template>
  <div class="grid min-w-0 gap-2.5">
    <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
      {{ $st('Model route') }}
      <OptionPicker
        :model-value="key"
        :options="routeOptions"
        :display-label="!key && partialRoute ? partialRoute : undefined"
        :title="$st('Model route')"
        :empty-label="$st('Inherit model selection')"
        :search-placeholder="$st('Search configured models...')"
        size="sm"
        :disabled="disabled"
        @update:model-value="chooseRoute"
      />
    </label>
    <div class="settings-control-grid">
      <label
        v-for="preference in preferences"
        :key="preference.key"
        class="grid min-w-0 gap-1 text-xs text-muted-foreground"
      >
        {{ preference.label }}
        <OptionPicker
          :model-value="preference.value"
          :options="preference.options"
          :title="preference.label"
          :allow-custom="true"
          :empty-label="$st('Inherit')"
          size="sm"
          :disabled="preferencesDisabled"
          @update:model-value="setPreference(preference.key, $event)"
        />
      </label>
      <label class="grid min-w-0 gap-1 text-xs text-muted-foreground">
        {{ $st('Parallel tools') }}
        <OptionPicker
          :model-value="
            typeof modelValue.parallel_tool_calls === 'boolean' ? String(modelValue.parallel_tool_calls) : ''
          "
          :options="parallelOptions"
          :title="$st('Parallel tools')"
          :empty-label="$st('Inherit')"
          size="sm"
          :disabled="preferencesDisabled"
          @update:model-value="setPreference('parallel_tool_calls', $event)"
        />
      </label>
    </div>
  </div>
</template>
