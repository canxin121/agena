import test from 'node:test'
import assert from 'node:assert/strict'

import {
  normalizeRememberedSettingsRoute,
  settingsTabFromRouteValue,
} from '../src/components/settings/sidebar/settingsSidebarNavigation'

test('legacy OpenCode settings links resolve to the corresponding Agena panel', () => {
  assert.equal(settingsTabFromRouteValue('/settings/opencode/providers'), 'providers')
  assert.equal(settingsTabFromRouteValue('/settings/opencode/permissions'), 'permissions')
  assert.equal(settingsTabFromRouteValue('/settings/opencode/plugins'), 'plugins')
  assert.equal(settingsTabFromRouteValue('/settings/opencode/activities'), 'activities')
  assert.equal(settingsTabFromRouteValue('/settings/opencode/memories'), 'memories')
  assert.equal(settingsTabFromRouteValue('/settings/opencode/usage'), 'usage')
})

test('removed Agent settings fall back to General instead of exposing an empty panel', () => {
  assert.equal(settingsTabFromRouteValue('/settings/opencode/agents'), 'general')
  assert.equal(normalizeRememberedSettingsRoute('/settings/opencode/agents'), '/settings/general')
})
