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
  for (const id of ['provider-studio', 'model-catalog', 'defaults', 'approval-model']) {
    assert.ok(catalog.includes(`id: '${id}'`), `missing models subpage ${id}`)
  }
  for (const id of [
    'plugin-workbench',
    'plugin-settings',
    'plugin-commands',
    'plugin-tools',
    'plugin-logs',
    'plugin-diagnostics',
    'marketplace',
    'mcp-server',
  ]) {
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
  assert.ok(!source.includes('useRouter'), 'content must not maintain a second route state')
  assert.ok(!source.includes('localStorage'), 'remembering the destination belongs to SettingsPage')
})

test('permission tool rules follow the shared source scope without a second scope picker', () => {
  const root = resolve(import.meta.dir, '../src/components/settings')
  const workbench = readFileSync(resolve(root, 'PermissionsWorkbenchPanel.vue'), 'utf8')
  const studio = readFileSync(resolve(root, 'PermissionStudioPanel.vue'), 'utf8')
  const rules = readFileSync(resolve(root, 'PermissionsPanel.vue'), 'utf8')

  assert.ok(workbench.includes('defineProps<{ activePage: string; scope: string }>()'))
  assert.ok(workbench.includes('<PermissionStudioPanel :scope="scope" :section="activePage" />'))
  assert.equal(workbench.includes('v-model:scope'), false)
  assert.doesNotMatch(workbench, /PermissionsPanel/)
  assert.ok(
    studio.includes('const selectedSource = computed<PermissionSource>(() => normalizePermissionSource(props.scope))'),
  )
  assert.equal(studio.includes(':options="sourceOptions"'), false)
  assert.ok(studio.includes('<PermissionsPanel v-if="selectedSection === \'tools\'" :scope="selectedSource"'))
  assert.ok(rules.includes("props.scope === 'workspace' && activeWorkspaceId.value !== null"))
  assert.ok(
    rules.includes('permissionRuleMatchesScope(rule, props.scope, activeWorkspaceId.value, activeSessionId.value)'),
  )
  assert.ok(rules.includes('scope: props.scope'))
  assert.doesNotMatch(rules, /newScope|scopeOptions/)
})

test('every layer-aware settings page reads its scope from the shared navigation', () => {
  const root = resolve(import.meta.dir, '../src/components/settings')
  const page = readFileSync(resolve(import.meta.dir, '../src/pages/SettingsPage.vue'), 'utf8')
  assert.ok(page.includes(':active-scope="activeScope"'))
  assert.ok(page.includes('(section) => buildSettingsSidebarNodes(section)'))
  assert.ok(page.includes('resolveSettingsScope(route.query.scope'))
  assert.ok(page.includes('settingsScopeStorageKey(activeSection.value)'))
  for (const section of ['models-providers', 'permissions', 'diagnostics']) {
    assert.match(page, new RegExp(`:scope="activeScope"\\s+v-else-if="activeSection === '${section}'"`))
  }

  const models = readFileSync(resolve(root, 'ModelsProvidersPanel.vue'), 'utf8')
  assert.ok(models.includes('<ApprovalSettingsPanel v-else-if="activePage === \'approval-model\'" :scope="scope" />'))
  assert.ok(models.includes('<ModelDefaultsPanel v-else :scope="scope" />'))

  // Content pages only report the layer they were opened with: none of them
  // renders a scope picker of its own.
  for (const file of [
    'PermissionStudioPanel.vue',
    'ModelDefaultsPanel.vue',
    'ApprovalSettingsPanel.vue',
    'AdvancedSettingsPanel.vue',
  ]) {
    const source = readFileSync(resolve(root, file), 'utf8')
    for (const options of [':options="sourceOptions"', ':options="scopeOptions"', ':options="layerOptions"']) {
      assert.equal(source.includes(options), false, `${file} must not render ${options}`)
    }
  }
})
