import test from 'node:test'
import assert from 'node:assert/strict'

import {
  normalizeRememberedSettingsRoute,
  settingsTabFromRouteValue,
} from '../src/components/settings/sidebar/settingsSidebarNavigation'

test('settings routes accept only current Agena section ids', () => {
  assert.equal(settingsTabFromRouteValue('/settings/models-providers'), 'models-providers')
  assert.equal(settingsTabFromRouteValue('/settings/permissions'), 'permissions')
  assert.equal(settingsTabFromRouteValue('/settings/plugins-tools'), 'plugins-tools')
  assert.equal(settingsTabFromRouteValue('/settings/runtime-session'), 'runtime-session')
  assert.equal(settingsTabFromRouteValue('/settings/interface'), 'interface')
  assert.equal(settingsTabFromRouteValue('/settings/diagnostics'), 'diagnostics')
})

test('unknown settings routes do not resolve as aliases', () => {
  assert.equal(settingsTabFromRouteValue('/settings/general'), null)
  assert.equal(settingsTabFromRouteValue('/settings/opencode/providers'), null)
  assert.equal(normalizeRememberedSettingsRoute('/settings/general'), '/settings/interface')
})
