import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const service = readFileSync(new URL('../src/lib/appTextPrompt.ts', import.meta.url), 'utf8')
const host = readFileSync(new URL('../src/components/AppTextPromptHost.vue', import.meta.url), 'utf8')
const app = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')

test('text prompts are queued and resolved without native browser prompt dialogs', () => {
  assert.match(service, /export function promptForText/)
  assert.match(service, /queue\.push/)
  assert.match(service, /export function resolveAppTextPrompt/)
})

test('text prompt host uses the shared FormDialog and accessible Input', () => {
  assert.match(host, /<FormDialog/)
  assert.match(host, /<Input/)
  assert.match(host, /autofocus/)
  assert.match(host, /:aria-label="appTextPromptRequest\.title"/)
  assert.match(host, /@keydown\.enter\.prevent="submit"/)
  assert.match(app, /<AppTextPromptHost \/>/)
})
