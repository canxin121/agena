import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('Plugin Marketplace uses server-owned GitHub search, lifecycle tasks, and install management', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/settings/PluginMarketplacePanel.vue'), 'utf8')

  for (const endpoint of [
    '/api/v1/plugins/marketplace/search',
    '/api/v1/plugins/marketplace/sync',
    '/api/v1/plugins/marketplace/installed',
    '/api/v1/plugins/marketplace/outdated',
    '/api/v1/plugins/marketplace/install',
    '/api/v1/plugins/marketplace/uninstall',
    '/api/v1/plugins/marketplace/upgrade',
    '/api/v1/runtime/tasks',
  ]) {
    assert.ok(source.includes(endpoint), `missing marketplace endpoint ${endpoint}`)
  }

  assert.ok(source.includes('owner/repository@v0.1.0'))
  assert.ok(source.includes('require_signature'))
  assert.ok(source.includes('allow_unverified'))
  assert.ok(source.includes('waitForTask'))
  assert.ok(source.includes("task.status === 'succeeded'"))
  assert.ok(source.includes('plugin.repository'))
  assert.ok(source.includes('plugin.tags'))
  assert.ok(source.includes('latest_source_commit'))
  assert.ok(source.includes('latest_source_repository'))
  assert.ok(source.includes('Release provenance'))
  assert.ok(source.includes('require_github_distribution'))
  assert.ok(source.includes('GitHub provenance required'))
  assert.ok(source.includes('Signature required'))
  assert.ok(source.includes('review_tier'))
  assert.ok(source.includes("$st('Official')"))
  assert.ok(source.includes("$st('Verified')"))
  assert.ok(source.includes("$st('Community')"))
  assert.ok(source.includes("$st('Featured')"))
})
