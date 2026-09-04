import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/tooltip.variants.ts', import.meta.url), 'utf8')

test('tooltips render above dialogs, option menus, confirms, and image viewer controls', () => {
  assert.match(source, /z-\[100\]/)
  assert.doesNotMatch(source, /\bz-50\b/)
})
