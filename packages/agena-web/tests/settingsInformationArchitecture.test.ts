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

  test.each([
    ['models-providers', 'provider-studio', 'ai-session', 'models'],
    ['models-providers', 'model-catalog', 'ai-session', 'models'],
    ['runtime-session', 'prompt-layers', 'ai-session', 'instructions'],
    ['runtime-session', 'client-versions', 'operations', 'clients'],
    ['runtime-session', 'compaction', 'ai-session', 'context'],
    ['plugins-tools', 'marketplace', 'extensions', 'plugins'],
    ['skills', 'install', 'extensions', 'skills'],
    ['mcp-server', 'connection', 'extensions', 'mcp-host'],
    ['permissions', 'overview', 'security', 'overview'],
    ['permissions', 'tools', 'security', 'execution'],
    ['interface', 'notifications', 'appearance', 'notifications'],
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

  test('search finds fields inside collapsed model tasks without exposing unrelated categories', () => {
    const groups = buildSettingsSidebarGroups({
      query: 'api key',
      tabs: categories,
      activeTab: 'interface',
      activeView: 'tui',
    })
    const rows = groups.flatMap((group) => group.items)
    expect(rows.some((row) => row.key === 'ai-session/models')).toBe(true)
    expect(rows.find((row) => row.key === 'ai-session/models')?.view).toBe('provider-studio')
    expect(rows.some((row) => row.key === 'ai-session/instructions')).toBe(false)
  })

  test('old section URLs and renamed model views remain valid', () => {
    for (const section of SETTINGS_TAB_IDS) {
      expect(settingsTabFromRouteValue(`/settings/${section}?view=any`)).toBe(section)
    }
    const pages = buildSettingsSubpages('models-providers')
    expect(resolveSettingsSubpage('defaults', '', pages, 'provider-studio')).toBe('model-settings')
  })
})
