import assert from 'node:assert/strict'
import test from 'node:test'

import {
  buildSettingsScopes,
  findSettingsScope,
  resolveSettingsScope,
  settingsScopeIdsForPage,
  settingsScopeStorageKey,
} from '../src/components/settings/settingsScopeCatalog'

test('every layer-aware settings page resolves the layers it can show', () => {
  for (const page of ['filesystem', 'network', 'tools']) {
    assert.deepEqual(settingsScopeIdsForPage('permissions', page), ['effective', 'global', 'workspace', 'session'])
  }
  for (const page of ['defaults', 'approval-model']) {
    assert.deepEqual(settingsScopeIdsForPage('models-providers', page), ['effective', 'global', 'workspace'])
  }
  assert.deepEqual(settingsScopeIdsForPage('diagnostics', 'advanced-settings'), ['global', 'workspace'])
})

test('pages without a layer choice resolve no scope', () => {
  assert.deepEqual(settingsScopeIdsForPage('interface', 'web-appearance'), [])
  assert.deepEqual(settingsScopeIdsForPage('diagnostics', 'usage'), [])
  assert.deepEqual(settingsScopeIdsForPage('plugins-tools', 'marketplace'), [])
  assert.deepEqual(buildSettingsScopes('runtime-session', 'compaction'), [])
})

test('a requested scope is reused only while the active page resolves it', () => {
  const permissionScopes = buildSettingsScopes('permissions', 'filesystem')
  const modelScopes = buildSettingsScopes('models-providers', 'defaults')

  assert.equal(resolveSettingsScope('global', '', permissionScopes), 'global')
  assert.equal(resolveSettingsScope('missing', 'session', permissionScopes), 'session')
  assert.equal(resolveSettingsScope('missing', 'missing', permissionScopes), 'effective')
  // A session layer cannot survive into a page that only resolves three layers.
  assert.equal(resolveSettingsScope('session', 'session', modelScopes), 'effective')
  assert.equal(resolveSettingsScope('global', '', []), '')
  assert.equal(resolveSettingsScope('', '', []), '')
})

test('scope memory is isolated by top-level section', () => {
  assert.equal(settingsScopeStorageKey('permissions'), 'studio.settings.scope.permissions')
  assert.notEqual(settingsScopeStorageKey('permissions'), settingsScopeStorageKey('models-providers'))
})

test('the active scope resolves its label and description from the shared catalog', () => {
  assert.deepEqual(
    buildSettingsScopes('diagnostics', 'advanced-settings').map((scope) => scope.id),
    ['global', 'workspace'],
  )
  assert.equal(findSettingsScope('diagnostics', 'advanced-settings', 'workspace')?.label, 'Workspace layer')
  assert.equal(findSettingsScope('permissions', 'tools', 'session')?.label, 'Current Session Permission')
  assert.equal(findSettingsScope('diagnostics', 'advanced-settings', 'session'), null)
  assert.equal(findSettingsScope('interface', 'tui', 'global'), null)
})
