<script setup lang="ts">
import type { ProviderModel, ProviderSummary } from '@/agena/lib/agenaApi'

const props = defineProps<{
  selectedProviderId: string
  selectedModelId: string
  selectedVariant: string
  providers: ProviderSummary[]
  providerDefaultModel: (providerId: string) => string
  providerModelOptions: (providerId: string) => ProviderModel[]
  providerModelLabel: (model: ProviderModel) => string
  modelVariantOptions: () => Array<{ id: string; label: string; description: string }>
}>()

const emit = defineEmits<{
  'update:selectedProviderId': [value: string]
  'update:selectedModelId': [value: string]
  'update:selectedVariant': [value: string]
}>()
</script>

<template>
  <section class="card">
    <h3>Run Options</h3>
    <div class="grid two">
      <div class="field">
        <label class="label" for="provider-select">Provider</label>
        <select
          id="provider-select"
          :value="props.selectedProviderId"
          class="select"
          @change="
            emit('update:selectedProviderId', ($event.target as HTMLSelectElement).value),
            emit('update:selectedModelId', props.providerDefaultModel(($event.target as HTMLSelectElement).value)),
            emit('update:selectedVariant', '')
          "
        >
          <option value="">Auto</option>
          <option v-for="provider in props.providers" :key="provider.provider_id" :value="provider.provider_id">
            {{ provider.provider_id }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="model-id">Model</label>
        <select
          id="model-id"
          :value="props.selectedModelId"
          class="select"
          @change="
            emit('update:selectedModelId', ($event.target as HTMLSelectElement).value),
            emit('update:selectedVariant', '')
          "
        >
          <option value="">Auto</option>
          <option
            v-for="model in props.providerModelOptions(props.selectedProviderId)"
            :key="`${model.provider_id}-${model.id}`"
            :value="model.id"
          >
            {{ props.providerModelLabel(model) }}
          </option>
        </select>
      </div>
      <div class="field">
        <label class="label" for="variant-id">Variant</label>
        <select
          id="variant-id"
          :value="props.selectedVariant"
          class="select"
          :disabled="props.modelVariantOptions().length === 0"
          @change="emit('update:selectedVariant', ($event.target as HTMLSelectElement).value)"
        >
          <option value="">Default</option>
          <option v-for="variant in props.modelVariantOptions()" :key="variant.id" :value="variant.id">
            {{ variant.label }}
          </option>
        </select>
      </div>
    </div>
  </section>
</template>
