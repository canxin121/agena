import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('Plugin Workbench exposes the complete Config/Tools/Commands/Capabilities/Logs/Diagnostics surface', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginsPanel.vue'), 'utf8')
  for (const tab of [
    'overview',
    'config',
    'tools',
    'commands',
    'views',
    'controls',
    'capabilities',
    'logs',
    'diagnostics',
  ]) {
    assert.ok(source.includes(`id: '${tab}'`), `missing plugin tab ${tab}`)
  }
  assert.ok(source.includes('<PluginConfigEditor'))
  assert.ok(source.includes("'/api/v1/plugins/ui/invoke-tool'"))
  assert.ok(source.includes('selectedTool.contract?.input_schema'))
  assert.ok(source.includes('selectedTool.permissions'))
  assert.ok(source.includes('diagnosticLogs'))
})
