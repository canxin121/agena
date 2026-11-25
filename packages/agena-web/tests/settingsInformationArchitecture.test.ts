import { describe, expect, test } from 'bun:test'
import { buildSettingsNavigationCategories } from '../src/components/settings/settingsInformationArchitecture'
import { buildSettingsSubpages } from '../src/components/settings/settingsNavigationCatalog'
import {
  SETTINGS_CATEGORY_IDS,
  SETTINGS_TAB_IDS,
  buildSettingsSidebarGroups,
  settingsTabFromRouteValue,
  type SettingsTab,
} from '../src/components/settings/sidebar/settingsSidebarNavigation'
import { resolveSettingsSubpage } from '../src/components/settings/workbench/settingsSectionNavigation'

const categories = buildSettingsNavigationCategories()

describe('task-based settings navigation', () => {
  test('presents six meaningful categories with at most one child level', () => {
    expect(categories.map((category) => category.id)).toEqual([...SETTINGS_CATEGORY_IDS])
    expect(categories.every((category) => category.group === 'all' && category.children?.length)).toBe(true)
    expect(categories.flatMap((category) => category.children || []).every((task) => !task.children?.length)).toBe(true)
  })

  test('every legacy settings subpage has exactly one user-facing task', () => {
    const expected = SETTINGS_TAB_IDS.flatMap((section) =>
      buildSettingsSubpages(section).map((page) => `${section}/${page.id}`),
    ).sort()
    const actual = categories
      .flatMap((category) =>
        (category.children || []).flatMap((task) =>
          (task.relatedViews || [task.view!]).map((view) => `${task.section}/${view}`),
        ),
      )
      .sort()
    expect(actual).toEqual(expected)
    expect(new Set(actual).size).toBe(actual.length)
  })

  test('AI & sessions puts Session settings first and Model Catalog last', () => {
    const rows = buildSettingsSidebarGroups({
      query: '',
      tabs: categories,
      activeTab: 'runtime-session',
      activeView: 'session-behavior',
      expandedNodeKeys: new Set(['ai-session']),
    }).flatMap((group) => group.items)
    expect(rows.filter((row) => row.key.startsWith('ai-session/')).map((row) => row.view)).toEqual([
      'session-behavior',
      'storage',
      'provider-studio',
      'agent-profiles',
      'system-prompt',
      'prompt-library',
      'model-catalog',
    ])
  })

  test.each([
    ['models-providers', 'provider-studio', 'ai-session', 'provider-studio'],
    ['models-providers', 'model-catalog', 'ai-session', 'model-catalog'],
    ['runtime-session', 'agent-profiles', 'ai-session', 'agent-profiles'],
    ['runtime-session', 'system-prompt', 'ai-session', 'system-prompt'],
    ['runtime-session', 'prompt-library', 'ai-session', 'prompt-library'],
    ['runtime-session', 'client-versions', 'operations', 'clients'],
    ['runtime-session', 'session-behavior', 'ai-session', 'session-behavior'],
    ['runtime-session', 'storage', 'ai-session', 'storage'],
    ['plugins-tools', 'marketplace', 'extensions', 'plugins'],
    ['skills', 'install', 'extensions', 'skills'],
    ['mcp-server', 'connection', 'extensions', 'mcp-host'],
    ['permissions', 'access', 'security', 'access'],
    ['interface', 'web-appearance', 'appearance', 'web'],
    ['diagnostics', 'usage', 'records', 'usage'],
    ['diagnostics', 'advanced-settings', 'operations', 'advanced'],
  ] as const)('highlights %s/%s under %s/%s', (section, view, categoryId, taskId) => {
    const groups = buildSettingsSidebarGroups({
      query: '',
      tabs: categories,
      activeTab: section,
      activeView: view,
      expandedNodeKeys: new Set(SETTINGS_CATEGORY_IDS),
    })
    const rows = groups.flatMap((group) => group.items)
    expect(rows.filter((row) => row.active).map((row) => row.key)).toEqual([`${categoryId}/${taskId}`])
    expect(rows.find((row) => row.key === categoryId)?.branchActive).toBe(true)
  })

  test.each([
    ['default model', 'session-behavior', 'session-behavior'],
    ['审批模型', 'session-behavior', 'session-behavior'],
    ['auxiliary model', 'session-behavior', 'session-behavior'],
    ['api key', 'provider-studio', 'provider-studio'],
    ['pricing', 'model-catalog', 'model-catalog'],
    ['指令', 'agent-profiles', 'agent-profiles'],
    ['提示词库', 'prompt-library', 'prompt-library'],
  ])('searching for %s opens its own sidebar page', (query, taskId, view) => {
    const groups = buildSettingsSidebarGroups({
      query,
      tabs: categories,
      activeTab: 'interface',
      activeView: 'tui',
    })
    const rows = groups.flatMap((group) => group.items)
    const aiRows = rows.filter((row) => row.key.startsWith('ai-session/') && row.view)
    expect(aiRows.map((row) => [row.key, row.view])).toEqual([[`ai-session/${taskId}`, view]])
  })

  test('Hook searches find the plugin settings workbench', () => {
    for (const query of ['hook', 'hooks', '钩子']) {
      const groups = buildSettingsSidebarGroups({
        query,
        tabs: categories,
        activeTab: 'interface',
        activeView: 'tui',
      })
      const rows = groups.flatMap((group) => group.items)
      expect(rows.some((row) => row.key === 'extensions/plugins')).toBe(true)
    }
  })

  test('default profile searches lead to Session settings', () => {
    for (const query of ['default profile', 'default_subagent_profile', '默认Profile', '默认档案', '子 Agent']) {
      const rows = buildSettingsSidebarGroups({
        query,
        tabs: categories,
        activeTab: 'interface',
        activeView: 'tui',
      }).flatMap((group) => group.items)
      expect(rows.find((row) => row.key === 'ai-session/session-behavior')?.view).toBe('session-behavior')
      expect(rows.some((row) => row.key === 'ai-session/agent-profiles')).toBe(false)
    }
  })

  test('old section URLs and remembered context settings remain valid', () => {
    for (const section of SETTINGS_TAB_IDS) {
      expect(settingsTabFromRouteValue(`/settings/${section}?view=any`)).toBe(section)
    }
    const pages = buildSettingsSubpages('runtime-session')
    expect(resolveSettingsSubpage('', 'compaction', pages, 'session-behavior')).toBe('session-behavior')
  })

  test('security searches permission domains and execution safeguards from one access editor', () => {
    const security = categories.find((category) => category.id === 'security')!
    const ids = security.children?.map((task) => task.id) || []
    expect(ids).toEqual(['profiles', 'assignments', 'access'])
    for (const query of ['filesystem', 'network', 'tools', 'sandbox', '文件', '网络', '工具', '资源预检']) {
      const rows = buildSettingsSidebarGroups({
        query,
        tabs: categories,
        activeTab: 'interface',
        activeView: 'tui',
      }).flatMap((group) => group.items)
      expect(rows.find((row) => row.key === 'security/access')?.view).toBe('access')
    }
  })
})
