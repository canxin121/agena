import assert from 'node:assert/strict'
import test from 'node:test'
import { buildSettingsSubpages } from '../src/components/settings/settingsNavigationCatalog'

import {
  filterSettingsSubpages,
  resolveSettingsSubpage,
  settingsSubpageStorageKey,
  type SettingsSubpageDefinition,
} from '../src/components/settings/workbench/settingsSectionNavigation'

const pages: SettingsSubpageDefinition[] = [
  { id: 'first', label: 'First page', description: 'Provider authentication', keywords: ['oauth'] },
  { id: 'second', label: 'Second page', description: 'Runtime diagnostics', keywords: ['tracing'] },
]

test('settings subpages prefer a valid deep link and fall back through remembered/default pages', () => {
  assert.equal(resolveSettingsSubpage('second', 'first', pages, 'first'), 'second')
  assert.equal(resolveSettingsSubpage('missing', 'second', pages, 'first'), 'second')
  assert.equal(resolveSettingsSubpage('missing', 'missing', pages, 'first'), 'first')
})

test('settings subpage search covers labels, descriptions, ids, and keywords', () => {
  assert.deepEqual(
    filterSettingsSubpages(pages, 'oauth').map((page) => page.id),
    ['first'],
  )
  assert.deepEqual(
    filterSettingsSubpages(pages, 'diagnostics').map((page) => page.id),
    ['second'],
  )
  assert.deepEqual(
    filterSettingsSubpages(pages, '').map((page) => page.id),
    ['first', 'second'],
  )
})

test('settings subpage memory keys are isolated by top-level section', () => {
  assert.equal(settingsSubpageStorageKey('models-providers'), 'studio.settings.subpage.models-providers')
  assert.notEqual(settingsSubpageStorageKey('permissions'), settingsSubpageStorageKey('plugins-tools'))
})

test('merged model settings preserve legacy deep links and remembered destinations', () => {
  const modelPages = buildSettingsSubpages('models-providers')
  for (const legacy of ['defaults', 'approval-model', ' APPROVAL-MODEL ']) {
    assert.equal(resolveSettingsSubpage(legacy, 'provider-studio', modelPages, 'provider-studio'), 'model-settings')
    assert.equal(resolveSettingsSubpage('', legacy, modelPages, 'provider-studio'), 'model-settings')
    assert.equal(resolveSettingsSubpage('model-catalog', legacy, modelPages, 'provider-studio'), 'model-catalog')
  }
  assert.deepEqual(
    filterSettingsSubpages(modelPages, 'approval-model').map((page) => page.id),
    ['model-settings'],
  )
})
