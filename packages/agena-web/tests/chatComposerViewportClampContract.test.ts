import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const layoutSource = readFileSync(new URL('../src/pages/chat/useChatComposerLayout.ts', import.meta.url), 'utf8')
const viewSource = readFileSync(new URL('../src/pages/chat/ChatPageView.vue', import.meta.url), 'utf8')

test('regular desktop composer height is clamped to preserve transcript space', () => {
  assert.match(layoutSource, /MIN_DESKTOP_TRANSCRIPT_HEIGHT = 160/)
  assert.match(layoutSource, /regularComposerMaxHeight/)
  assert.match(layoutSource, /regularComposerTargetHeight/)
  assert.match(layoutSource, /height - MIN_DESKTOP_TRANSCRIPT_HEIGHT/)
  assert.match(viewSource, /:max-height="composerMaxHeight"/)
})

test('composer switches cleanly between compact and desktop layouts without losing the desktop preference', () => {
  assert.match(layoutSource, /watch\([\s\S]*ui\.isCompactLayout/)
  assert.match(layoutSource, /composerTargetHeight\.value = DEFAULT_MOBILE_COMPOSER_HEIGHT/)
  assert.doesNotMatch(layoutSource, /composerUserHeight\.value = DEFAULT_MOBILE_COMPOSER_HEIGHT/)
  assert.match(layoutSource, /syncRegularComposerHeight\(\)/)
  assert.match(viewSource, /:min-height="ui\.isCompactLayout \? 160 : 190"/)
})
