import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const service = readFileSync(new URL('../src/lib/appConfirm.ts', import.meta.url), 'utf8')
const host = readFileSync(new URL('../src/components/AppConfirmHost.vue', import.meta.url), 'utf8')
const app = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')

test('global app confirmation requests are queued and resolved through one promise service', () => {
  assert.match(service, /export function confirmAction/)
  assert.match(service, /queue\.push/)
  assert.match(service, /pumpConfirmQueue/)
  assert.match(service, /export function resolveAppConfirm/)
})

test('confirmation host reuses the shared Radix-backed ConfirmPopover above every app state', () => {
  assert.match(host, /<ConfirmPopover/)
  assert.match(host, /force-dialog/)
  assert.match(host, /@confirm="resolveAppConfirm\(true\)"/)
  assert.match(app, /<AppConfirmHost \/>/)
})
