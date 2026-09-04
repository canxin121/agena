import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const style = readFileSync(new URL('../src/style.css', import.meta.url), 'utf8')

test('coarse-pointer sidebars use larger stable touch targets without changing desktop density', () => {
  assert.match(style, /:root\.touch-pointer \.oc-vscode-icon-button[\s\S]*min-width: 2rem;[\s\S]*min-height: 2rem;/)
  assert.match(style, /:root\.touch-pointer \.oc-icon-button[\s\S]*min-width: 2rem;[\s\S]*min-height: 2rem;/)
  assert.match(style, /:root\.touch-pointer \[data-oc-list-item-frame\][\s\S]*min-height: 2\.25rem;/)
})

test('hover-only row actions remain reachable on touch devices', () => {
  assert.match(
    style,
    /:root\.touch-pointer \[data-oc-list-item-frame\] \.oc-list-item-actions[\s\S]*max-width: 100%;[\s\S]*opacity: 1;[\s\S]*pointer-events: auto;/,
  )
  assert.match(style, /:root\.touch-pointer \.oc-vscode-row-actions[\s\S]*pointer-events-auto opacity-100/)
})
