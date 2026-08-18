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
  assert.ok(source.includes('{{ option.summary }}'))
})
