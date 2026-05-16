<script setup lang="ts">
import type {
  MarketplaceInstalledPluginResource,
  MarketplaceOutdatedPluginResource,
  MarketplacePluginResource,
} from '@/agena/lib/agenaApi'

const props = defineProps<{
  marketplaceRegistryUrl: string
  marketplaceRegistryId: string
  marketplaceQuery: string
  marketplaceLoading: boolean
  marketplaceInstallSpec: string
  marketplaceAllowUnverified: boolean
  marketplaceRequireSignature: boolean
  marketplaceRefreshIndex: boolean
  marketplaceCascadeUninstall: boolean
  filteredMarketplacePlugins: MarketplacePluginResource[]
  marketplacePlugins: MarketplacePluginResource[]
  marketplaceInstalled: MarketplaceInstalledPluginResource[]
  marketplaceOutdated: MarketplaceOutdatedPluginResource[]
  installedMarketplacePluginIds: Set<string>
  searchMarketplaceAction: (options?: { refresh?: boolean }) => void | Promise<void>
  syncMarketplaceRegistryAction: () => void | Promise<void>
  upgradeMarketplacePluginAction: (pluginId?: string) => void | Promise<void>
  installMarketplacePluginAction: (pluginId?: string) => void | Promise<void>
  uninstallMarketplacePluginAction: (pluginId: string) => void | Promise<void>
}>()

const emit = defineEmits<{
  'update:marketplaceRegistryUrl': [value: string]
  'update:marketplaceRegistryId': [value: string]
  'update:marketplaceQuery': [value: string]
  'update:marketplaceInstallSpec': [value: string]
  'update:marketplaceAllowUnverified': [value: boolean]
  'update:marketplaceRequireSignature': [value: boolean]
  'update:marketplaceRefreshIndex': [value: boolean]
  'update:marketplaceCascadeUninstall': [value: boolean]
}>()
</script>

<template>
  <div class="grid two">
    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Marketplace</h3>
          <p class="muted">
            Search a registry, install plugins into the active config, and trigger upgrade or uninstall flows without
            leaving the web UI.
          </p>
        </div>
        <span class="badge">live</span>
      </div>

      <div class="settings-summary" style="margin-top: 12px">
        <div class="summary-item">
          <div class="summary-label">Registry</div>
          <div class="summary-value">{{ props.marketplacePlugins.length }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Filtered</div>
          <div class="summary-value">{{ props.filteredMarketplacePlugins.length }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Installed</div>
          <div class="summary-value">{{ props.marketplaceInstalled.length }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Outdated</div>
          <div class="summary-value">{{ props.marketplaceOutdated.length }}</div>
        </div>
      </div>

      <div class="grid two" style="margin-top: 12px">
        <div class="field">
          <label class="label" for="plugin-marketplace-registry-url">Registry URL</label>
          <input
            id="plugin-marketplace-registry-url"
            :value="props.marketplaceRegistryUrl"
            class="input mono"
            placeholder="https://example.com/registry.json"
            @input="emit('update:marketplaceRegistryUrl', ($event.target as HTMLInputElement).value)"
          />
        </div>
        <div class="field">
          <label class="label" for="plugin-marketplace-registry-id">Registry ID</label>
          <input
            id="plugin-marketplace-registry-id"
            :value="props.marketplaceRegistryId"
            class="input mono"
            placeholder="default"
            @input="emit('update:marketplaceRegistryId', ($event.target as HTMLInputElement).value)"
          />
        </div>
      </div>

      <div class="field" style="margin-top: 12px">
        <label class="label" for="plugin-marketplace-search">Search</label>
        <input
          id="plugin-marketplace-search"
          :value="props.marketplaceQuery"
          class="input mono"
          placeholder="plugin id / name / description"
          @input="emit('update:marketplaceQuery', ($event.target as HTMLInputElement).value)"
          @keyup.enter="props.searchMarketplaceAction()"
        />
      </div>

      <div class="button-row" style="margin-top: 12px; flex-wrap: wrap">
        <button
          class="button primary"
          :disabled="props.marketplaceLoading || !props.marketplaceRegistryUrl.trim()"
          @click="props.searchMarketplaceAction()"
        >
          Search Registry
        </button>
        <button
          class="button"
          :disabled="props.marketplaceLoading || !props.marketplaceRegistryUrl.trim()"
          @click="props.searchMarketplaceAction({ refresh: true })"
        >
          Refresh Search
        </button>
        <button
          class="button"
          :disabled="props.marketplaceLoading || !props.marketplaceRegistryUrl.trim()"
          @click="props.syncMarketplaceRegistryAction"
        >
          Sync Registry
        </button>
        <button
          class="button"
          :disabled="props.marketplaceLoading || !props.marketplaceInstalled.length"
          @click="props.upgradeMarketplacePluginAction()"
        >
          Upgrade All Installed
        </button>
      </div>

      <div class="grid two" style="margin-top: 16px">
        <div class="field">
          <label class="label" for="plugin-marketplace-install-spec">Install Spec</label>
          <input
            id="plugin-marketplace-install-spec"
            :value="props.marketplaceInstallSpec"
            class="input mono"
            placeholder="plugin-id or plugin-id@1.2.3"
            @input="emit('update:marketplaceInstallSpec', ($event.target as HTMLInputElement).value)"
          />
        </div>
        <div class="stack" style="justify-content: end">
          <label class="muted"
            ><input
              :checked="props.marketplaceAllowUnverified"
              type="checkbox"
              @change="emit('update:marketplaceAllowUnverified', ($event.target as HTMLInputElement).checked)"
            />
            allow unverified</label
          >
          <label class="muted"
            ><input
              :checked="props.marketplaceRequireSignature"
              type="checkbox"
              @change="emit('update:marketplaceRequireSignature', ($event.target as HTMLInputElement).checked)"
            />
            require signature</label
          >
          <label class="muted"
            ><input
              :checked="props.marketplaceRefreshIndex"
              type="checkbox"
              @change="emit('update:marketplaceRefreshIndex', ($event.target as HTMLInputElement).checked)"
            />
            refresh index on install</label
          >
          <label class="muted"
            ><input
              :checked="props.marketplaceCascadeUninstall"
              type="checkbox"
              @change="emit('update:marketplaceCascadeUninstall', ($event.target as HTMLInputElement).checked)"
            />
            cascade uninstall</label
          >
        </div>
      </div>

      <div class="button-row" style="margin-top: 12px; flex-wrap: wrap">
        <button
          class="button primary"
          :disabled="
            props.marketplaceLoading || !props.marketplaceRegistryUrl.trim() || !props.marketplaceInstallSpec.trim()
          "
          @click="props.installMarketplacePluginAction"
        >
          Install Plugin
        </button>
      </div>

      <div v-if="props.filteredMarketplacePlugins.length" class="list" style="margin-top: 16px">
        <div v-for="plugin in props.filteredMarketplacePlugins" :key="plugin.plugin_id" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div>
                <strong>{{ plugin.plugin_id }}</strong>
              </div>
              <div class="muted">{{ plugin.name || plugin.description || 'No description' }}</div>
              <div class="muted mono">
                latest={{ plugin.latest_version || 'n/a' }} · kind={{ plugin.latest_kind || 'n/a' }} · platform={{
                  plugin.latest_platform || 'n/a'
                }}
                · versions={{ plugin.version_count }}
              </div>
              <div v-if="plugin.homepage" class="muted mono">{{ plugin.homepage }}</div>
            </div>
            <span class="badge">{{
              props.installedMarketplacePluginIds.has(plugin.plugin_id) ? 'installed' : 'registry'
            }}</span>
          </div>
          <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button
              class="button primary"
              :disabled="props.marketplaceLoading || !props.marketplaceRegistryUrl.trim()"
              @click="props.installMarketplacePluginAction(plugin.plugin_id)"
            >
              Install
            </button>
            <button
              class="button"
              :disabled="props.marketplaceLoading"
              @click="emit('update:marketplaceInstallSpec', plugin.plugin_id)"
            >
              Use Spec
            </button>
            <button
              class="button"
              :disabled="props.marketplaceLoading || !props.installedMarketplacePluginIds.has(plugin.plugin_id)"
              @click="props.upgradeMarketplacePluginAction(plugin.plugin_id)"
            >
              Update
            </button>
            <button
              class="button danger"
              :disabled="props.marketplaceLoading || !props.installedMarketplacePluginIds.has(plugin.plugin_id)"
              @click="props.uninstallMarketplacePluginAction(plugin.plugin_id)"
            >
              Uninstall
            </button>
          </div>
        </div>
      </div>
      <p v-else-if="props.marketplaceLoading" class="muted" style="margin-top: 12px">Loading marketplace…</p>
      <p v-else class="muted" style="margin-top: 12px">Set a registry URL and search to load marketplace entries.</p>
    </section>

    <section class="card">
      <h3>Registry / Installed State</h3>
      <div class="stack">
        <div><strong>Registry Entries:</strong> {{ props.marketplacePlugins.length }}</div>
        <div><strong>Installed via Marketplace:</strong> {{ props.marketplaceInstalled.length }}</div>
        <div><strong>Outdated:</strong> {{ props.marketplaceOutdated.length }}</div>
      </div>

      <div v-if="props.marketplaceInstalled.length" class="list" style="margin-top: 12px">
        <div v-for="plugin in props.marketplaceInstalled" :key="plugin.plugin_id" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div>
                <strong>{{ plugin.plugin_id }}</strong>
              </div>
              <div class="muted">v{{ plugin.version }} · {{ plugin.kind }} · {{ plugin.platform }}</div>
              <div class="muted mono">config={{ plugin.config_path }}</div>
              <div v-if="plugin.registry_url" class="muted mono">registry={{ plugin.registry_url }}</div>
            </div>
            <span class="badge">installed</span>
          </div>
          <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button
              class="button"
              :disabled="props.marketplaceLoading"
              @click="props.upgradeMarketplacePluginAction(plugin.plugin_id)"
            >
              Update
            </button>
            <button
              class="button"
              :disabled="props.marketplaceLoading"
              @click="emit('update:marketplaceInstallSpec', plugin.plugin_id)"
            >
              Use Spec
            </button>
            <button
              class="button danger"
              :disabled="props.marketplaceLoading"
              @click="props.uninstallMarketplacePluginAction(plugin.plugin_id)"
            >
              Uninstall
            </button>
          </div>
        </div>
      </div>
      <p v-else class="muted" style="margin-top: 12px">No marketplace-installed plugins recorded yet.</p>

      <div v-if="props.marketplaceOutdated.length" class="list" style="margin-top: 16px">
        <div v-for="plugin in props.marketplaceOutdated" :key="plugin.plugin_id" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div>
                <strong>{{ plugin.plugin_id }}</strong>
              </div>
              <div class="muted">installed {{ plugin.installed_version }} → latest {{ plugin.latest_version }}</div>
            </div>
            <button
              class="button primary"
              :disabled="props.marketplaceLoading"
              @click="props.upgradeMarketplacePluginAction(plugin.plugin_id)"
            >
              Update Now
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
