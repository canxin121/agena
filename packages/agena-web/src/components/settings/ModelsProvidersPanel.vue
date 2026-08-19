<script setup lang="ts">
import ModelCatalogPanel from '@/components/settings/ModelCatalogPanel.vue'
import ProviderStudioPanel from '@/components/settings/ProviderStudioPanel.vue'
import ProvidersPanel from '@/components/settings/ProvidersPanel.vue'
import SettingsSectionWorkbench from '@/components/settings/workbench/SettingsSectionWorkbench.vue'
import type { SettingsSubpageDefinition } from '@/components/settings/workbench/settingsSectionNavigation'
import { settingsText as st } from '@/i18n/settingsText'

const pages: SettingsSubpageDefinition[] = [
  {
    id: 'provider-studio',
    label: st('Provider Studio'),
    description: st('Create providers, configure authentication, adapters, and model routes.'),
    keywords: ['provider', 'authentication', 'oauth', 'api key', 'adapter', 'model'],
  },
  {
    id: 'defaults',
    label: st('Model defaults'),
    description: st('Choose the runtime default and the automatic permission approval model.'),
    keywords: ['default model', 'approval', 'thinking', 'speed', 'verbosity'],
  },
  {
    id: 'model-catalog',
    label: st('Model Catalog'),
    description: st('Search the resolved model catalog and inspect capabilities, limits, modes, and pricing.'),
    keywords: ['catalog', 'metadata', 'pricing', 'capabilities', 'context window'],
  },
  {
    id: 'inventory',
    label: st('Configured inventory'),
    description: st('Review every configured provider, adapter, endpoint, and model.'),
    keywords: ['inventory', 'configured', 'provider list', 'adapter list'],
  },
]
</script>

<template>
  <SettingsSectionWorkbench
    section="models-providers"
    :title="$st('Models & Providers')"
    :description="
      $st(
        'Manage provider credentials and adapters, select default model routes, and inspect the complete model catalog.',
      )
    "
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
