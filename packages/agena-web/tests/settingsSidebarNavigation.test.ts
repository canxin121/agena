import assert from 'node:assert/strict'
import test from 'node:test'

import {
  buildSettingsSidebarGroups,
  buildSettingsSidebarTabs,
  type SettingsSidebarTab,
} from '../src/components/settings/sidebar/settingsSidebarNavigation'

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
