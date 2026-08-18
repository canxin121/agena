import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('app keeps the previous page unmounted until auth status is known', () => {
  const appSource = readFileSync(resolve(import.meta.dir, '../src/App.vue'), 'utf8')
  const authSource = readFileSync(resolve(import.meta.dir, '../src/stores/auth.ts'), 'utf8')

  assert.match(appSource, /showLoading = computed\(\(\) => health\.data === null \|\| !auth\.checked\)/)
  assert.match(authSource, /return \{[\s\S]*checked,[\s\S]*needsLogin,/)
})
