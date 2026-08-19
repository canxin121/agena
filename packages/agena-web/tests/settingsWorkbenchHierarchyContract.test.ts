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

test('every dense settings domain is split into searchable subpages', () => {
  for (const [name, relativePath] of Object.entries(components)) {
    const source = readFileSync(resolve(import.meta.dir, relativePath), 'utf8')
    assert.ok(source.includes('<SettingsSectionWorkbench'), `${name} must use the section workbench`)
    assert.ok(source.includes('default-page='), `${name} must define a stable default page`)
  }
})

test('models and plugin workbenches expose the high-value parity subpages', () => {
  const models = readFileSync(resolve(import.meta.dir, components.models), 'utf8')
  for (const id of ['provider-studio', 'defaults', 'model-catalog', 'inventory']) {
    assert.ok(models.includes(`id: '${id}'`), `missing models subpage ${id}`)
  }

  const plugins = readFileSync(resolve(import.meta.dir, components.plugins), 'utf8')
  for (const id of ['plugin-workbench', 'mcp-server', 'harnesses']) {
    assert.ok(plugins.includes(`id: '${id}'`), `missing plugins subpage ${id}`)
  }
})
