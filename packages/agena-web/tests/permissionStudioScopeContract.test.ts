import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('permission studio follows the layer the shared navigation forwarded', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PermissionStudioPanel.vue'), 'utf8')
  assert.ok(
    source.includes('const selectedSource = computed<PermissionSource>(() => normalizePermissionSource(props.scope))'),
  )
  assert.ok(
    source.includes("return value === 'global' || value === 'workspace' || value === 'session' ? value : 'effective'"),
  )
  assert.ok(source.includes("findSettingsScope('permissions', props.section, selectedSource.value)?.label"))
  assert.ok(source.includes('v-if="selectedSource === \'session\' && !activeSessionId"'))
  // The layer is named once in the section header, so the page must not repeat
  // it with its own picker or per-layer summaries.
  assert.equal(source.includes('sourceOptions'), false)
  assert.equal(source.includes('permissionSummary'), false)
  assert.equal(source.includes('globalConfig'), false)
  assert.equal(source.includes('workspaceConfig'), false)
  assert.equal(source.includes('sessionConfigSnapshot'), false)
})
