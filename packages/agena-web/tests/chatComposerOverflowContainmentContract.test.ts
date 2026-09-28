import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const viewSource = readFileSync(resolve(import.meta.dir, '../src/pages/chat/ChatPageView.vue'), 'utf8')
const composerSource = readFileSync(resolve(import.meta.dir, '../src/components/chat/Composer.vue'), 'utf8')

test('a long draft scrolls inside the composer editor instead of pushing the toolbar off screen', () => {
  // `min-height: min-content` let the content height expand the shell past the
  // resizable pane, which pushed the control row below the viewport. The shell
  // must stay shrinkable so the editor owns the overflow instead.
  assert.match(viewSource, /class="flex-1 shrink-0 sm:shrink min-h-0"/)
  assert.doesNotMatch(viewSource, /min-h-min/)
})

test('the composer editor keeps an internal scroll container', () => {
  assert.match(composerSource, /overflow-y-auto/)
})
