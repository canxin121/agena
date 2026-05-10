import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createPluginsMarketplacePanelState, usePluginsMarketplacePageState } from './usePluginsMarketplacePageState'

describe('usePluginsMarketplacePageState', () => {
  test('assembles marketplace panel state from provided plugins source', () => {
    const marketplace = createPluginsMarketplacePanelState({
      filteredMarketplacePlugins: computed(() => []),
      installMarketplacePluginAction: async () => {},
      installedMarketplacePluginIds: computed(() => new Set<string>()),
      marketplaceAllowUnverified: ref(false),
      marketplaceCascadeUninstall: ref(false),
      marketplaceInstallSpec: ref(''),
      marketplaceInstalled: ref([]),
      marketplaceLoading: ref(false),
      marketplaceOutdated: ref([]),
      marketplacePlugins: ref([]),
      marketplaceQuery: ref(''),
      marketplaceRefreshIndex: ref(false),
      marketplaceRegistryId: ref(''),
      marketplaceRegistryUrl: ref(''),
      marketplaceRequireSignature: ref(false),
      searchMarketplaceAction: async () => {},
      syncMarketplaceRegistryAction: async () => {},
      uninstallMarketplacePluginAction: async () => {},
      upgradeMarketplacePluginAction: async () => {},
    })

    expect(marketplace.query.value).toBe('')
    expect(marketplace.loading.value).toBe(false)
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/plugins/marketplace' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = usePluginsMarketplacePageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'plugins' })
          return {
            shared,
            state: {
              filteredMarketplacePlugins: computed(() => []),
              installMarketplacePluginAction: async () => {},
              installedMarketplacePluginIds: computed(() => new Set<string>()),
              marketplaceAllowUnverified: ref(false),
              marketplaceCascadeUninstall: ref(false),
              marketplaceInstallSpec: ref(''),
              marketplaceInstalled: ref([]),
              marketplaceLoading: ref(false),
              marketplaceOutdated: ref([]),
              marketplacePlugins: ref([]),
              marketplaceQuery: ref(''),
              marketplaceRefreshIndex: ref(false),
              marketplaceRegistryId: ref(''),
              marketplaceRegistryUrl: ref(''),
              marketplaceRequireSignature: ref(false),
              searchMarketplaceAction: async () => {},
              syncMarketplaceRegistryAction: async () => {},
              uninstallMarketplacePluginAction: async () => {},
              upgradeMarketplacePluginAction: async () => {},
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.marketplace.query.value).toBe('')
  })
})
