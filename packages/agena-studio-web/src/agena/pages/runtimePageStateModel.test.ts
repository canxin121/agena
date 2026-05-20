import { describe, expect, test } from 'bun:test'

import type {
  MarketplaceInstalledPluginResource,
  MarketplacePluginResource,
  RuntimeSkill,
  SessionResource,
  WorkspaceResource,
} from '@/agena/lib/agenaApi'
import {
  buildRuntimeSectionPath,
  defaultTabForSection,
  filterMarketplacePluginsByQuery,
  filterRuntimeSkillsByQuery,
  resolveRuntimeTabFromRoute,
  sanitizeRuntimeSectionQuery,
  isPluginsTab,
  isRuntimeTab,
  isSettingsTab,
  normalizeOptionalText,
  normalizePort,
  pickSessionId,
  pickWorkspaceId,
  pluginsTabs,
  queryMatchesText,
  resolveRuntimeRouteSection,
  resolveRuntimeTabFromPath,
  runtimeTabs,
  sectionBasePaths,
  sectionDescriptions,
  sectionNavItems,
  sectionPageLoaders,
  sectionPagePaths,
  sectionTabNavigationItems,
  sectionTitles,
  settingsTabs,
} from './runtimePageStateModel'

function sampleWorkspaces(): WorkspaceResource[] {
  return [
    { id: 1, path: '/a', created_at: 'x', updated_at: 'x', session_count: 1 },
    { id: 2, path: '/b', created_at: 'x', updated_at: 'x', session_count: 0 },
  ]
}

function sampleSessions(): SessionResource[] {
  return [
    { id: 11, workspace_id: 1, title: 'one', version: 1, created_at: 'x', updated_at: 'x', message_count: 0, child_session_count: 0 },
    { id: 12, workspace_id: 1, title: 'two', version: 1, created_at: 'x', updated_at: 'x', message_count: 0, child_session_count: 0 },
  ]
}

function samplePlugins(): MarketplacePluginResource[] {
  return [
    {
      plugin_id: 'alpha',
      name: 'Alpha Plugin',
      description: 'fast local runtime helper',
      homepage: null,
      version_count: 2,
      latest_version: '1.0.0',
      latest_kind: 'native',
      latest_platform: 'linux',
    },
    {
      plugin_id: 'beta-market',
      name: 'Beta Market',
      description: 'registry auth failed helper',
      homepage: null,
      version_count: 4,
      latest_version: '2.0.0',
      latest_kind: 'python',
      latest_platform: 'any',
    },
  ]
}

function sampleInstalled(): MarketplaceInstalledPluginResource[] {
  return [
    {
      plugin_id: 'beta-market',
      version: '1.5.0',
      kind: 'python',
      platform: 'any',
      binary_path: '/tmp/beta',
      config_path: '/tmp/config.json',
      sha256: null,
      installed_at: 'x',
      registry_id: 'default',
      registry_url: 'https://example.com/registry.json',
      archive_extracted: false,
    },
  ]
}

function sampleRuntimeSkills(): RuntimeSkill[] {
  return [
    {
      name: 'deploy-check',
      description: 'verify release readiness',
      aliases: ['ship', 'release'],
      source_path: '.agena/skills/deploy-check.md',
    },
    {
      name: 'trace-runtime',
      description: 'inspect runtime traces',
      aliases: ['trace'],
      source_path: null,
    },
  ]
}

describe('runtimePageStateModel', () => {
  test('resolveRuntimeRouteSection respects explicit section and path', () => {
    expect(resolveRuntimeRouteSection('/runtime')).toBe('runtime')
    expect(resolveRuntimeRouteSection('/plugins')).toBe('plugins')
    expect(resolveRuntimeRouteSection('/settings/desktop')).toBe('settings')
    expect(resolveRuntimeRouteSection('/runtime', 'settings')).toBe('settings')
  })

  test('runtime route helpers map sections to concrete subpaths', () => {
    expect(resolveRuntimeTabFromPath('/runtime')).toBe('overview')
    expect(resolveRuntimeTabFromPath('/runtime/lsp')).toBe('lsp')
    expect(resolveRuntimeTabFromPath('/settings/desktop')).toBe('desktop')
    expect(resolveRuntimeTabFromPath('/plugins/marketplace')).toBe('marketplace')
    expect(resolveRuntimeTabFromRoute('/runtime', { tab: 'skills' }, 'runtime')).toBe('skills')
    expect(resolveRuntimeTabFromRoute('/settings', { settingsTab: 'desktop' }, 'settings')).toBe('desktop')
    expect(resolveRuntimeTabFromRoute('/plugins', { pluginsTab: 'marketplace' }, 'plugins')).toBe('marketplace')
    expect(sanitizeRuntimeSectionQuery({ workspace: '1', tab: 'workflow', settingsTab: 'permissions', pluginsTab: 'installed' })).toEqual({ workspace: '1' })
    expect(buildRuntimeSectionPath('runtime', 'skills')).toBe('/runtime/skills')
    expect(buildRuntimeSectionPath('settings', 'desktop')).toBe('/settings/desktop')
    expect(buildRuntimeSectionPath('plugins', 'installed')).toBe('/plugins/installed')
  })

  test('section tab registries and guards stay in sync', () => {
    expect(runtimeTabs.map((tab) => tab.id)).toEqual(['overview', 'workflow', 'mcp', 'lsp', 'skills', 'operator'])
    expect(settingsTabs.map((tab) => tab.id)).toEqual(['providers', 'permissions', 'desktop'])
    expect(pluginsTabs.map((tab) => tab.id)).toEqual(['installed', 'marketplace'])
    expect(defaultTabForSection('runtime')).toBe('overview')
    expect(defaultTabForSection('settings')).toBe('providers')
    expect(defaultTabForSection('plugins')).toBe('installed')
    expect(sectionTitles).toEqual({ runtime: 'Runtime', plugins: 'Plugins', settings: 'Settings' })
    expect(sectionBasePaths).toEqual({ runtime: '/runtime', plugins: '/plugins', settings: '/settings' })
    expect(sectionPagePaths).toEqual({ runtime: './RuntimePage.vue', plugins: './PluginsPage.vue', settings: './SettingsPage.vue' })
    expect(typeof sectionPageLoaders.runtime).toBe('function')
    expect(typeof sectionPageLoaders.plugins).toBe('function')
    expect(typeof sectionPageLoaders.settings).toBe('function')
    expect(sectionNavItems).toEqual([
      { section: 'runtime', label: 'Runtime' },
      { section: 'plugins', label: 'Plugins' },
      { section: 'settings', label: 'Settings' },
    ])
    expect(sectionTabNavigationItems.map((item) => item.id)).toEqual([
      'nav.runtime.overview',
      'nav.runtime.workflow',
      'nav.runtime.mcp',
      'nav.runtime.lsp',
      'nav.runtime.skills',
      'nav.runtime.operator',
      'nav.plugins.installed',
      'nav.plugins.marketplace',
      'nav.settings.providers',
      'nav.settings.permissions',
      'nav.settings.desktop',
    ])
    expect(sectionTabNavigationItems.find((item) => item.id === 'nav.runtime.workflow')?.slash).toBe('/runtime-workflow')
    expect(sectionTabNavigationItems.find((item) => item.id === 'nav.runtime.workflow')?.shortcutSlash).toBe('/workflow')
    expect(sectionTabNavigationItems.find((item) => item.id === 'nav.settings.desktop')?.tab).toBe('desktop')
    expect(sectionTabNavigationItems.find((item) => item.id === 'nav.settings.desktop')?.shortcutSlash).toBe('/desktop')
    expect(sectionDescriptions.runtime.includes('runtime state')).toBe(true)
    expect(sectionDescriptions.plugins.includes('installed plugins')).toBe(true)
    expect(sectionDescriptions.settings.includes('providers')).toBe(true)
    expect(isRuntimeTab('skills')).toBe(true)
    expect(isRuntimeTab('desktop')).toBe(false)
    expect(isSettingsTab('desktop')).toBe(true)
    expect(isSettingsTab('marketplace')).toBe(false)
    expect(isPluginsTab('installed')).toBe(true)
    expect(isPluginsTab('overview')).toBe(false)
  })

  test('pickWorkspaceId and pickSessionId preserve valid selections', () => {
    expect(pickWorkspaceId(2, sampleWorkspaces())).toBe(2)
    expect(pickWorkspaceId(99, sampleWorkspaces())).toBe(1)
    expect(pickSessionId(12, sampleSessions())).toBe(12)
    expect(pickSessionId(99, sampleSessions())).toBe(11)
  })

  test('normalize helpers coerce optional text and ports', () => {
    expect(normalizeOptionalText('  value  ')).toBe('value')
    expect(normalizeOptionalText('   ')).toBe(null)
    expect(normalizePort('3211', 3210)).toBe(3211)
    expect(normalizePort('-2', 3210)).toBe(3210)
    expect(normalizePort('abc', 3210)).toBe(3210)
  })

  test('filterMarketplacePluginsByQuery matches registry and installed metadata', () => {
    expect(filterMarketplacePluginsByQuery(samplePlugins(), sampleInstalled(), '').map((plugin) => plugin.plugin_id)).toEqual([
      'alpha',
      'beta-market',
    ])
    expect(filterMarketplacePluginsByQuery(samplePlugins(), sampleInstalled(), 'market').map((plugin) => plugin.plugin_id)).toEqual([
      'beta-market',
    ])
    expect(filterMarketplacePluginsByQuery(samplePlugins(), sampleInstalled(), 'installed').map((plugin) => plugin.plugin_id)).toEqual([
      'beta-market',
    ])
  })

  test('queryMatchesText handles empty and normalized text queries', () => {
    expect(queryMatchesText(['Alpha', 'Beta'], '')).toBe(true)
    expect(queryMatchesText(['Alpha', null, 'Beta Path'], 'beta path')).toBe(true)
    expect(queryMatchesText(['Alpha', 'LSP --stdio', '.ts', 'pnpm'], 'stdio')).toBe(true)
    expect(queryMatchesText(['Alpha', 'Beta'], 'gamma')).toBe(false)
  })

  test('filterRuntimeSkillsByQuery matches name aliases description and source path', () => {
    expect(filterRuntimeSkillsByQuery(sampleRuntimeSkills(), '').map((entry) => entry.name)).toEqual([
      'deploy-check',
      'trace-runtime',
    ])
    expect(filterRuntimeSkillsByQuery(sampleRuntimeSkills(), 'release').map((entry) => entry.name)).toEqual([
      'deploy-check',
    ])
    expect(filterRuntimeSkillsByQuery(sampleRuntimeSkills(), 'trace').map((entry) => entry.name)).toEqual([
      'trace-runtime',
    ])
    expect(filterRuntimeSkillsByQuery(sampleRuntimeSkills(), '.agena/skills').map((entry) => entry.name)).toEqual([
      'deploy-check',
    ])
  })
})
