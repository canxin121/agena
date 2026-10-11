import assert from 'node:assert/strict'
import test from 'node:test'

import {
  buildSettingsSidebarGroups,
  buildSettingsSidebarTabs,
  type SettingsSidebarTab,
} from '../src/components/settings/sidebar/settingsSidebarNavigation'
import { buildSettingsSidebarNodes } from '../src/components/settings/settingsNavigationCatalog'

const tabs: SettingsSidebarTab[] = [
  {
    id: 'plugins-tools',
    label: 'Plugins & Tools',
    icon: 'plugins-tools',
    group: 'core',
    keywords: ['plugins'],
    children: [
      {
        id: 'plugin-workbench',
        label: 'Plugin Workbench',
        description: 'Configure plugins and inspect logs.',
        view: 'plugin-workbench',
      },
      {
        id: 'integrations',
        label: 'Integrations',
        children: [
          {
            id: 'mcp-server',
            label: 'MCP Server',
            description: 'OAuth listener and public identity.',
            keywords: ['oauth'],
            view: 'mcp-server',
          },
        ],
      },
    ],
  },
]

function rows(args: Parameters<typeof buildSettingsSidebarGroups>[0]) {
  return buildSettingsSidebarGroups(args).flatMap((group) => group.items)
}

const scopedTabs: SettingsSidebarTab[] = [
  {
    id: 'permissions',
    label: 'Permissions',
    icon: 'permissions',
    group: 'core',
    children: [
      {
        id: 'filesystem',
        label: 'Filesystem',
        description: 'File and path access.',
        children: [
          { id: 'effective', label: 'Effective Permission', view: 'filesystem', scope: 'effective' },
          { id: 'global', label: 'Global Permission', view: 'filesystem', scope: 'global' },
          { id: 'workspace', label: 'Workspace Permission', view: 'filesystem', scope: 'workspace' },
          { id: 'session', label: 'Current Session Permission', view: 'filesystem', scope: 'session' },
        ],
      },
      { id: 'network', label: 'Network', description: 'Network access.', view: 'network' },
    ],
  },
]

test('settings sidebar expands recursive navigation nodes with stable depth and destinations', () => {
  assert.deepEqual(
    rows({ query: '', tabs, activeTab: 'interface' }).map((row) => row.label),
    ['Plugins & Tools'],
  )

  const expanded = rows({
    query: '',
    tabs,
    activeTab: 'interface',
    expandedNodeKeys: new Set(['plugins-tools', 'plugins-tools/integrations']),
  })
  assert.deepEqual(
    expanded.map((row) => [row.label, row.depth, row.view || '']),
    [
      ['Plugins & Tools', 0, ''],
      ['Plugin Workbench', 1, 'plugin-workbench'],
      ['Integrations', 1, ''],
      ['MCP Server', 2, 'mcp-server'],
    ],
  )
})

test('settings sidebar marks the active leaf and each ancestor branch', () => {
  const activeRows = rows({
    query: '',
    tabs,
    activeTab: 'plugins-tools',
    activeView: 'mcp-server',
    expandedNodeKeys: new Set(['plugins-tools', 'plugins-tools/integrations']),
  })

  assert.equal(activeRows.find((row) => row.label === 'MCP Server')?.active, true)
  assert.equal(activeRows.find((row) => row.label === 'Integrations')?.branchActive, true)
  assert.equal(activeRows.find((row) => row.label === 'Plugins & Tools')?.branchActive, true)
  assert.equal(activeRows.filter((row) => row.active).length, 1)
})

test('settings sidebar search keeps and expands ancestors of matching descendants', () => {
  const matchingRows = rows({
    query: 'oauth',
    tabs,
    activeTab: 'interface',
  })

  assert.deepEqual(
    matchingRows.map((row) => [row.label, row.expanded]),
    [
      ['Plugins & Tools', true],
      ['Integrations', true],
      ['MCP Server', false],
    ],
  )
})

test('settings sidebar tab builders assign destinations only to recursive leaves', () => {
  const [tab] = buildSettingsSidebarTabs(
    () => 'Plugins & Tools',
    () => [
      {
        id: 'integrations',
        label: 'Integrations',
        children: [{ id: 'mcp-server', label: 'MCP Server' }],
      },
    ],
  )

  assert.equal(tab?.children?.[0]?.view, undefined)
  assert.equal(tab?.children?.[0]?.children?.[0]?.view, 'mcp-server')
})

test('settings sidebar expands scope rows under the page that owns them', () => {
  const expanded = rows({
    query: '',
    tabs: scopedTabs,
    activeTab: 'permissions',
    activeView: 'filesystem',
    activeScope: 'workspace',
    expandedNodeKeys: new Set(['permissions', 'permissions/filesystem']),
  })

  assert.deepEqual(
    expanded.map((row) => [row.label, row.depth, row.view || '', row.scope || '']),
    [
      ['Permissions', 0, '', ''],
      ['Filesystem', 1, '', ''],
      ['Effective Permission', 2, 'filesystem', 'effective'],
      ['Global Permission', 2, 'filesystem', 'global'],
      ['Workspace Permission', 2, 'filesystem', 'workspace'],
      ['Current Session Permission', 2, 'filesystem', 'session'],
      ['Network', 1, 'network', ''],
    ],
  )

  assert.deepEqual(
    expanded.filter((row) => row.active).map((row) => row.label),
    ['Workspace Permission'],
  )
  assert.equal(expanded.find((row) => row.label === 'Filesystem')?.branchActive, true)
  assert.equal(expanded.find((row) => row.label === 'Permissions')?.branchActive, true)
  assert.equal(expanded.find((row) => row.label === 'Global Permission')?.active, false)
  assert.equal(expanded.find((row) => row.label === 'Global Permission')?.branchActive, false)
})

test('settings sidebar scope rows light up only for the selected layer', () => {
  const activeRows = rows({
    query: '',
    tabs: scopedTabs,
    activeTab: 'permissions',
    activeView: 'filesystem',
    activeScope: 'session',
    expandedNodeKeys: new Set(['permissions', 'permissions/filesystem']),
  })

  assert.deepEqual(
    activeRows.filter((row) => row.active).map((row) => row.label),
    ['Current Session Permission'],
  )
  assert.equal(activeRows.find((row) => row.label === 'Workspace Permission')?.active, false)

  // A collapsed branch keeps highlighting the page it belongs to.
  const collapsed = rows({
    query: '',
    tabs: scopedTabs,
    activeTab: 'permissions',
    activeView: 'filesystem',
    activeScope: 'session',
    expandedNodeKeys: new Set(['permissions']),
  })
  assert.deepEqual(
    collapsed.map((row) => row.label),
    ['Permissions', 'Filesystem', 'Network'],
  )
  assert.equal(collapsed.filter((row) => row.active).length, 0)
  assert.equal(collapsed.find((row) => row.label === 'Filesystem')?.branchActive, true)
})

test('settings sidebar search reaches scope rows', () => {
  const matching = rows({ query: 'session', tabs: scopedTabs, activeTab: 'interface' })
  assert.deepEqual(
    matching.map((row) => row.label),
    ['Permissions', 'Filesystem', 'Current Session Permission'],
  )
})

test('the settings catalog attaches scope rows to layer-aware pages only', () => {
  const permissionNodes = buildSettingsSidebarNodes('permissions')
  assert.deepEqual(
    permissionNodes
      .find((node) => node.id === 'filesystem')
      ?.children?.map((child) => [child.id, child.view, child.scope]),
    [
      ['effective', 'filesystem', 'effective'],
      ['global', 'filesystem', 'global'],
      ['workspace', 'filesystem', 'workspace'],
      ['session', 'filesystem', 'session'],
    ],
  )
  assert.equal(permissionNodes.find((node) => node.id === 'network')?.children?.length, 4)
  assert.equal(permissionNodes.find((node) => node.id === 'tools')?.children?.length, 4)
  assert.deepEqual(
    buildSettingsSidebarNodes('models-providers')
      .find((node) => node.id === 'defaults')
      ?.children?.map((child) => child.scope),
    ['effective', 'global', 'workspace'],
  )
  assert.deepEqual(
    buildSettingsSidebarNodes('diagnostics')
      .find((node) => node.id === 'advanced-settings')
      ?.children?.map((child) => child.scope),
    ['global', 'workspace'],
  )
  // Pages without a layer choice stay plain destinations.
  assert.equal(
    buildSettingsSidebarNodes('models-providers').find((node) => node.id === 'provider-studio')?.children,
    undefined,
  )
  assert.equal(
    buildSettingsSidebarNodes('interface').every((node) => !node.children),
    true,
  )
})
