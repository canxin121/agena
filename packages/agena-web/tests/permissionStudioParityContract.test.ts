import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('Permission Studio covers every TUI policy layer and editable rule family', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PermissionStudioPanel.vue'), 'utf8')
  for (const value of ['global', 'workspace', 'session', 'effective']) {
    assert.ok(source.includes(`value: '${value}'`), `missing permission source ${value}`)
  }
  for (const value of ['allow', 'auto', 'ask', 'deny']) {
    assert.ok(source.includes(`value: '${value}'`), `missing permission mode ${value}`)
  }
  for (const capability of [
    'setPathDefault',
    'renamePathRule',
    'setNetworkMode',
    'renameNetworkRule',
    'setToolDefault',
    'renameToolName',
    'renameCommandRule',
    'Raw PermissionConfig',
  ]) {
    assert.ok(source.includes(capability), `missing Permission Studio capability ${capability}`)
  }
})
