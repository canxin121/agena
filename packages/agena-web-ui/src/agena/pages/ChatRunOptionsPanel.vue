<script setup lang="ts">
import type { ProviderModel, ProviderSummary } from '@/agena/lib/agenaApi'

const props = defineProps<{
  selectedProviderId: string
  selectedAdapterId: string
  selectedModelId: string
  selectedThinkingMode: string
  selectedSpeedMode: string
  selectedVerbosity: string
  selectedParallelToolCalls: string
  selectedTemperature: string
  selectedMaxOutput: string
  selectedSystemPrompt: string
  providers: ProviderSummary[]
  providerDefaultAdapter: (providerId: string) => string
  providerDefaultModel: (providerId: string) => string
  providerAdapterOptions: (providerId: string) => string[]
  providerModelOptions: (providerId: string, adapterId?: string) => ProviderModel[]
  providerModelLabel: (model: ProviderModel) => string
  modelThinkingModeOptions: () => Array<{ id: string; label: string; description: string }>
  modelSpeedModeOptions: () => Array<{ id: string; label: string; description: string }>
  modelVerbosityOptions: () => Array<{ id: string; label: string; description: string }>
  modelParallelToolCallsOptions: () => Array<{ id: string; label: string; description: string }>
}>()

const emit = defineEmits<{
  'update:selectedProviderId': [value: string]
  'update:selectedAdapterId': [value: string]
  'update:selectedModelId': [value: string]
  'update:selectedThinkingMode': [value: string]
  'update:selectedSpeedMode': [value: string]
  'update:selectedVerbosity': [value: string]
  'update:selectedParallelToolCalls': [value: string]
  'update:selectedTemperature': [value: string]
  'update:selectedMaxOutput': [value: string]
  'update:selectedSystemPrompt': [value: string]
}>()
</script>

<template>
  <section id="chat-run-options" class="card" tabindex="-1">
    <h3>Run Options</h3>
    <div class="grid two">
      <div class="field">
        <label class="label" for="provider-select">Provider</label>
        <select
          id="provider-select"
          :value="props.selectedProviderId"
          class="select"
          @input="
            (emit('update:selectedProviderId', ($event.target as HTMLSelectElement).value),
            emit('update:selectedAdapterId', props.providerDefaultAdapter(($event.target as HTMLSelectElement).value)),
            emit('update:selectedModelId', props.providerDefaultModel(($event.target as HTMLSelectElement).value)),
            emit('update:selectedThinkingMode', ''),
            emit('update:selectedSpeedMode', ''),
            emit('update:selectedVerbosity', ''),
            emit('update:selectedParallelToolCalls', ''))
          "
        >
          <option value="">Auto</option>
          <option v-for="provider in props.providers" :key="provider.provider_id" :value="provider.provider_id">
            {{ provider.provider_id }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="adapter-id">Adapter</label>
        <select
          id="adapter-id"
          :value="props.selectedAdapterId"
          class="select"
          @input="
            (emit('update:selectedAdapterId', ($event.target as HTMLSelectElement).value),
            emit('update:selectedThinkingMode', ''),
            emit('update:selectedSpeedMode', ''),
            emit('update:selectedVerbosity', ''),
            emit('update:selectedParallelToolCalls', ''))
          "
        >
          <option value="">Auto</option>
          <option
            v-for="adapterId in props.providerAdapterOptions(props.selectedProviderId)"
            :key="adapterId"
            :value="adapterId"
          >
            {{ adapterId }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="model-id">Model</label>
        <select
          id="model-id"
          :value="props.selectedModelId"
          class="select"
          @input="
            (emit('update:selectedModelId', ($event.target as HTMLSelectElement).value),
            emit('update:selectedThinkingMode', ''),
            emit('update:selectedSpeedMode', ''),
            emit('update:selectedVerbosity', ''),
            emit('update:selectedParallelToolCalls', ''))
          "
        >
          <option value="">Auto</option>
          <option
            v-for="model in props.providerModelOptions(props.selectedProviderId, props.selectedAdapterId)"
            :key="`${model.provider_id}-${model.adapter_id || 'default'}-${model.id}`"
            :value="model.id"
          >
            {{ props.providerModelLabel(model) }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="thinking-mode-id">Thinking</label>
        <select
          id="thinking-mode-id"
          :value="props.selectedThinkingMode"
          class="select"
          :disabled="props.modelThinkingModeOptions().length === 0"
          @input="emit('update:selectedThinkingMode', ($event.target as HTMLSelectElement).value)"
        >
          <option value="">Default</option>
          <option
            v-for="thinkingMode in props.modelThinkingModeOptions()"
            :key="thinkingMode.id"
            :value="thinkingMode.id"
          >
            {{ thinkingMode.label }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="speed-mode-id">Speed</label>
        <select
          id="speed-mode-id"
          :value="props.selectedSpeedMode"
          class="select"
          :disabled="props.modelSpeedModeOptions().length === 0"
          @input="emit('update:selectedSpeedMode', ($event.target as HTMLSelectElement).value)"
        >
          <option value="">Default</option>
          <option v-for="speedMode in props.modelSpeedModeOptions()" :key="speedMode.id" :value="speedMode.id">
            {{ speedMode.label }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="verbosity-id">Verbosity</label>
        <select
          id="verbosity-id"
          :value="props.selectedVerbosity"
          class="select"
          :disabled="props.modelVerbosityOptions().length === 0"
          @input="emit('update:selectedVerbosity', ($event.target as HTMLSelectElement).value)"
        >
          <option value="">Default</option>
          <option v-for="verbosity in props.modelVerbosityOptions()" :key="verbosity.id" :value="verbosity.id">
            {{ verbosity.label }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="parallel-tool-calls-id">Parallel Tools</label>
        <select
          id="parallel-tool-calls-id"
          :value="props.selectedParallelToolCalls"
          class="select"
          :disabled="props.modelParallelToolCallsOptions().length === 0"
          @input="emit('update:selectedParallelToolCalls', ($event.target as HTMLSelectElement).value)"
        >
          <option value="">Default</option>
          <option v-for="option in props.modelParallelToolCallsOptions()" :key="option.id" :value="option.id">
            {{ option.label }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="temperature-id">Temperature</label>
        <input
          id="temperature-id"
          :value="props.selectedTemperature"
          class="input"
          type="number"
          step="any"
          placeholder="Model default"
          @input="emit('update:selectedTemperature', ($event.target as HTMLInputElement).value)"
        />
        <span class="muted">Optional sampling override. Leave empty to use the model default.</span>
      </div>
      <div class="field">
        <label class="label" for="max-output-id">Max Output Tokens</label>
        <input
          id="max-output-id"
          :value="props.selectedMaxOutput"
          class="input"
          type="number"
          min="1"
          step="1"
          placeholder="Model default"
          @input="emit('update:selectedMaxOutput', ($event.target as HTMLInputElement).value)"
        />
        <span class="muted">Optional positive token limit for the next model run.</span>
      </div>
    </div>
    <div class="field run-options-system-field">
      <label class="label" for="system-prompt-id">System Prompt Override</label>
      <textarea
        id="system-prompt-id"
        :value="props.selectedSystemPrompt"
        class="textarea"
        rows="5"
        placeholder="Leave empty to use the agent and runtime system prompt"
        @input="emit('update:selectedSystemPrompt', ($event.target as HTMLTextAreaElement).value)"
      />
      <span class="muted">Applied to send, continue, compact, and aside runs.</span>
    </div>
  </section>
</template>
