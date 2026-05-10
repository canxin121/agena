<script setup lang="ts">
import type { usePluginsPageState } from './usePluginsPageState'

import PluginsMarketplacePanel from './PluginsMarketplacePanel.vue'

const props = defineProps<{
  marketplace: ReturnType<typeof usePluginsPageState>['panels']['marketplace']
}>()
</script>

<template>
  <PluginsMarketplacePanel
    :marketplace-registry-url="props.marketplace.registryUrl"
    :marketplace-registry-id="props.marketplace.registryId"
    :marketplace-query="props.marketplace.query"
    :marketplace-loading="props.marketplace.loading"
    :marketplace-install-spec="props.marketplace.installSpec"
    :marketplace-allow-unverified="props.marketplace.allowUnverified"
    :marketplace-require-signature="props.marketplace.requireSignature"
    :marketplace-refresh-index="props.marketplace.refreshIndex"
    :marketplace-cascade-uninstall="props.marketplace.cascadeUninstall"
    :filtered-marketplace-plugins="props.marketplace.filteredPlugins"
    :marketplace-plugins="props.marketplace.plugins"
    :marketplace-installed="props.marketplace.installed"
    :marketplace-outdated="props.marketplace.outdated"
    :installed-marketplace-plugin-ids="props.marketplace.installedPluginIds"
    :search-marketplace-action="props.marketplace.searchAction"
    :sync-marketplace-registry-action="props.marketplace.syncRegistryAction"
    :upgrade-marketplace-plugin-action="props.marketplace.upgradeAction"
    :install-marketplace-plugin-action="props.marketplace.installAction"
    :uninstall-marketplace-plugin-action="props.marketplace.uninstallAction"
    @update:marketplace-registry-url="props.marketplace.registryUrl = $event"
    @update:marketplace-registry-id="props.marketplace.registryId = $event"
    @update:marketplace-query="props.marketplace.query = $event"
    @update:marketplace-install-spec="props.marketplace.installSpec = $event"
    @update:marketplace-allow-unverified="props.marketplace.allowUnverified = $event"
    @update:marketplace-require-signature="props.marketplace.requireSignature = $event"
    @update:marketplace-refresh-index="props.marketplace.refreshIndex = $event"
    @update:marketplace-cascade-uninstall="props.marketplace.cascadeUninstall = $event"
  />
</template>
