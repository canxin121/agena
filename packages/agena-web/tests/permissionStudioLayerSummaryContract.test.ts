import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('permission studio compares global, workspace, session, and effective layer summaries', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PermissionStudioPanel.vue'), 'utf8')
  assert.ok(source.includes('globalConfig'))
  assert.ok(source.includes('workspaceConfig'))
  assert.ok(source.includes('sessionConfigSnapshot'))
  assert.ok(source.includes('effectiveConfig'))
  assert.ok(source.includes('function permissionSummary'))
  // The source is chosen in the shared left navigation, so the page reports the
  // layer it was opened with instead of rendering a second picker.
  assert.ok(source.includes('sourceOptions.find((option) => option.value === selectedSource)?.label'))
  assert.ok(source.includes('option.value === selectedSource)?.summary'))
  assert.equal(source.includes(':options="sourceOptions"'), false)
})
