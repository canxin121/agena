import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const components = {
  models: '../src/components/settings/ModelsProvidersPanel.vue',
  permissions: '../src/components/settings/PermissionsWorkbenchPanel.vue',
  plugins: '../src/components/settings/PluginsToolsPanel.vue',
  runtime: '../src/components/settings/RuntimeSessionPanel.vue',
  diagnostics: '../src/components/settings/DiagnosticsWorkbenchPanel.vue',
}

test('every dense settings domain uses the shared subpage catalog and stable workbench defaults', () => {
  for (const [name, relativePath] of Object.entries(components)) {
    const source = readFileSync(resolve(import.meta.dir, relativePath), 'utf8')
    assert.ok(source.includes('<SettingsSectionWorkbench'), `${name} must use the section workbench`)
    assert.ok(source.includes('buildSettingsSubpages('), `${name} must use the shared subpage catalog`)
    assert.ok(source.includes('default-page='), `${name} must define a stable default page`)
  }
})

test('the shared catalog exposes the high-value models and plugin subpages', () => {
  const catalog = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/settingsNavigationCatalog.ts'),
    'utf8',
  )
  for (const id of ['provider-studio', 'model-catalog', 'defaults']) {
    assert.ok(catalog.includes(`id: '${id}'`), `missing models subpage ${id}`)
  }
  for (const id of ['plugin-workbench', 'marketplace', 'mcp-server']) {
    assert.ok(catalog.includes(`id: '${id}'`), `missing plugins subpage ${id}`)
  }
  for (const id of ['filesystem', 'network', 'tools']) {
    assert.ok(catalog.includes(`id: '${id}'`), `missing permission subpage ${id}`)
  }
  assert.ok(!catalog.includes("id: 'inventory'"), 'configured inventory belongs in Provider Studio')
  assert.ok(!catalog.includes("id: 'harnesses'"), 'tool harness settings are removed')
})

test('the section workbench renders only the selected content, not a second navigation', () => {
  const source = readFileSync(
    resolve(import.meta.dir, '../src/components/settings/workbench/SettingsSectionWorkbench.vue'),
    'utf8',
  )
  assert.ok(!source.includes('<aside'), 'settings content must not render a second desktop navigation')
  assert.ok(!source.includes('SearchInput'), 'section-local navigation search must be removed')
  assert.ok(!source.includes('OptionPicker'), 'compact layout must use the shared left navigation')
  assert.ok(source.includes('<slot :active-page="activePage"'))
})

test('permission tool rules follow the shared source scope without a second scope picker', () => {
  const root = resolve(import.meta.dir, '../src/components/settings')
  const workbench = readFileSync(resolve(root, 'PermissionsWorkbenchPanel.vue'), 'utf8')
  const studio = readFileSync(resolve(root, 'PermissionStudioPanel.vue'), 'utf8')
  const rules = readFileSync(resolve(root, 'PermissionsPanel.vue'), 'utf8')

  assert.ok(workbench.includes('v-model:scope="permissionScope"'))
  assert.doesNotMatch(workbench, /PermissionsPanel/)
  assert.ok(studio.includes('<PermissionsPanel v-if="selectedSection === \'tools\'" :scope="selectedSource"'))
  assert.ok(rules.includes("props.scope === 'workspace' && activeWorkspaceId.value !== null"))
  assert.ok(
    rules.includes('permissionRuleMatchesScope(rule, props.scope, activeWorkspaceId.value, activeSessionId.value)'),
  )
  assert.ok(rules.includes('scope: props.scope'))
  assert.doesNotMatch(rules, /newScope|scopeOptions/)
})
