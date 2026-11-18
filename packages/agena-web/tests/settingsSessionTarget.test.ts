import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import { resolveSettingsSessionId, settingsSessionId } from '../src/components/settings/settingsSessionScope'

const compiled = transformSync(
  readFileSync(new URL('../src/components/settings/settingsSessionScope.ts', import.meta.url), 'utf8'),
  { loader: 'ts', format: 'cjs' },
).code

function targetFixture() {
  const recorded = vue.ref('7')
  const route = vue.reactive({
    path: '/settings/permissions',
    query: { scope: 'session', view: 'access', windowId: 'settings-pane' } as Record<string, string>,
    hash: '#permission-rules-command',
  })
  const chat = vue.reactive({
    selectedSessionId: '4',
    getSessionById: (id: string) => ({ id, title: 'Conversation ' + id }),
    getSessionDirectory: (id: string) => '/fixture/' + id,
  })
  const replacements: Array<Record<string, unknown>> = []
  let allowNavigation = true
  const modules: Record<string, unknown> = {
    vue,
    'vue-router': {
      useRoute: () => route,
      useRouter: () => ({
        async replace(destination: typeof route) {
          replacements.push(structuredClone(destination))
          if (!allowNavigation) return { type: 4 }
          Object.assign(route, destination)
        },
      }),
    },
    '@/i18n/settingsText': { settingsText: (value: string) => value },
    '@/lib/lastSession': { lastSessionId: recorded },
    '@/stores/chat': { useChatStoreUnscoped: () => chat },
  }
  const module = { exports: {} as { useSettingsSessionTarget: () => any } }
  new Function('require', 'module', 'exports', compiled)(
    (id: string) => {
      if (!(id in modules)) throw new Error('Unexpected session target dependency: ' + id)
      return modules[id]
    },
    module,
    module.exports,
  )
  const target = module.exports.useSettingsSessionTarget()
  return {
    target,
    route,
    recorded,
    chat,
    replacements,
    blockNavigation: () => {
      allowNavigation = false
    },
  }
}

describe('session-scoped settings target', () => {
  test('accepts one positive integer conversation id', () => {
    expect(settingsSessionId('42')).toBe('42')
    expect(settingsSessionId(42)).toBe('42')
    expect(settingsSessionId(' 7 ')).toBe('7')
    expect(settingsSessionId('007')).toBe('7')
  })

  test('rejects anything that is not a conversation id', () => {
    expect(settingsSessionId('')).toBe('')
    expect(settingsSessionId(null)).toBe('')
    expect(settingsSessionId(undefined)).toBe('')
    expect(settingsSessionId('abc')).toBe('')
    expect(settingsSessionId('0')).toBe('')
    expect(settingsSessionId('-1')).toBe('')
    expect(settingsSessionId('1.5')).toBe('')
  })

  test('takes the first usable entry of a repeated query value', () => {
    expect(settingsSessionId(['', '12'])).toBe('12')
  })

  test('an explicit settings target takes precedence over the last conversation', () => {
    expect(resolveSettingsSessionId('42', '7')).toBe('42')
    expect(resolveSettingsSessionId(['', '0042'], '7')).toBe('42')
    expect(resolveSettingsSessionId(undefined, '7')).toBe('7')
    expect(resolveSettingsSessionId('invalid', '7')).toBe('7')
    expect(resolveSettingsSessionId(undefined, undefined)).toBe('')
  })

  test('choosing the current fallback pins it without changing chat or the last-conversation record', async () => {
    const fixture = targetFixture()
    expect(fixture.target.sessionId.value).toBe('7')
    await fixture.target.selectSession('007')
    expect(fixture.route.query.targetSessionId).toBe('7')
    expect(fixture.replacements).toHaveLength(1)
    fixture.recorded.value = '9'
    expect(fixture.target.sessionId.value).toBe('7')
    expect(fixture.target.selected.value.title).toBe('Conversation 7')
    expect(fixture.chat.selectedSessionId).toBe('4')
  })

  test('selection preserves this settings pane and section while dropping the retired chat query', async () => {
    const fixture = targetFixture()
    fixture.route.query.sessionId = '999'
    await fixture.target.selectSession('42')
    expect(fixture.route).toEqual({
      path: '/settings/permissions',
      query: { scope: 'session', view: 'access', windowId: 'settings-pane', targetSessionId: '42' },
      hash: '#permission-rules-command',
    })
    expect(fixture.target.sessionId.value).toBe('42')
    expect(fixture.recorded.value).toBe('7')
    expect(fixture.chat.selectedSessionId).toBe('4')
    await fixture.target.selectSession('42')
    await fixture.target.selectSession('not-a-session')
    expect(fixture.replacements).toHaveLength(1)
  })

  test('a rejected route transition leaves the selected target and its label unchanged', async () => {
    const fixture = targetFixture()
    fixture.blockNavigation()
    await fixture.target.selectSession('42')
    expect(fixture.target.sessionId.value).toBe('7')
    expect(fixture.target.selected.value.title).toBe('Conversation 7')
    expect(fixture.route.query.targetSessionId).toBeUndefined()
    expect(fixture.recorded.value).toBe('7')
  })
})
