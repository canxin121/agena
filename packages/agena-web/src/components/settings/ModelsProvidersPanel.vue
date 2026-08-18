<script setup lang="ts">
import ModelCatalogPanel from '@/components/settings/ModelCatalogPanel.vue'
import ProviderStudioPanel from '@/components/settings/ProviderStudioPanel.vue'
import ProvidersPanel from '@/components/settings/ProvidersPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import type { SettingsSubpageDefinition } from '@/components/settings/workbench/settingsSectionNavigation'

const pages: SettingsSubpageDefinition[] = [
  {
    id: 'provider-studio',
    label: 'Provider Studio',
    description: 'Create providers, configure authentication, adapters, and model routes.',
    keywords: ['provider', 'authentication', 'oauth', 'api key', 'adapter', 'model'],
  },
  {
    id: 'defaults',
    label: 'Model defaults',
    description: 'Choose the runtime default and the automatic permission approval model.',
    keywords: ['default model', 'approval', 'thinking', 'speed', 'verbosity'],
  },
  {
    id: 'model-catalog',
    label: 'Model Catalog',
    description: 'Search the resolved model catalog and inspect capabilities, limits, modes, and pricing.',
    keywords: ['catalog', 'metadata', 'pricing', 'capabilities', 'context window'],
  },
  {
    id: 'inventory',
    label: 'Configured inventory',
    description: 'Review every configured provider, adapter, endpoint, and model.',
    keywords: ['inventory', 'configured', 'provider list', 'adapter list'],
  },
]
</script>

<template>
  <SettingsSectionWorkbench
    section="models-providers"
    title="Models & Providers"
    description="Manage provider credentials and adapters, select default model routes, and inspect the complete model catalog."
    :pages="pages"
    default-page="provider-studio"
    v-slot="{ activePage }"
  >
    <ProviderStudioPanel v-if="activePage === 'provider-studio'" />
    <ProvidersPanel v-else-if="activePage === 'defaults'" view="defaults" />
    <ModelCatalogPanel v-else-if="activePage === 'model-catalog'" />
    <ProvidersPanel v-else view="inventory" />
  </SettingsSectionWorkbench>
</template>
