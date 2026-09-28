import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

const chatPage = read('../src/pages/ChatPage.vue')
const chatPageView = read('../src/pages/chat/ChatPageView.vue')
const transcriptVim = read('../src/pages/chat/useChatTranscriptVim.ts')
const settingsPage = read('../src/pages/SettingsPage.vue')
const helpDialog = read('../src/components/HelpDialog.vue')

test('the transcript Vim mode is a stored preference combined with pane focus', () => {
  assert.ok(chatPage.includes('resolveTranscriptVimEnabled(settings.data?.chatTranscriptVim, ui.isMobileDevice)'))
  assert.ok(chatPage.includes('enabled: computed(() => transcriptVimEnabled.value && isFocusedWorkspacePane.value)'))
})

test('vim chrome is hidden while the mode is off', () => {
  assert.ok(chatPageView.includes('v-if="transcriptVimEnabled"'))
  assert.ok(chatPageView.includes('openTranscriptSearch(true)'))
  assert.ok(chatPageView.includes('jumpTranscriptSearch(true)'))
  assert.ok(chatPageView.includes('jumpTranscriptSearch(false)'))
})

test('the composable gates every visual side effect on the mode', () => {
  assert.ok(transcriptVim.includes('function vimActive(): boolean'))
  assert.ok(transcriptVim.includes('if (!vimActive()) return'))
  assert.ok(transcriptVim.includes('if (!vimActive() || !key) return'))
  assert.ok(transcriptVim.includes('() => (opts.enabled ? opts.enabled.value : true),'))
  assert.ok(transcriptVim.includes('openSearch,'))
  assert.ok(transcriptVim.includes('jumpSearch,'))
})

test('the settings page exposes the switch and the help dialog follows it', () => {
  assert.ok(settingsPage.includes('v-model="chatTranscriptVim"'))
  assert.ok(settingsPage.includes("t('settings.appearance.chat.transcriptVim')"))
  assert.ok(settingsPage.includes("t('settings.appearance.chat.transcriptVimHint')"))
  assert.ok(helpDialog.includes('vimOnly: true'))
  assert.ok(helpDialog.includes('resolveTranscriptVimEnabled(settings.data?.chatTranscriptVim, ui.isMobileDevice)'))
})
