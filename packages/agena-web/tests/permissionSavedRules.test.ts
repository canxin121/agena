import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

import { permissionRuleMatchesScope } from '../src/lib/permissionRuleScope'

const source = readFileSync(new URL('../src/components/settings/PermissionsPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'saved-network-rules-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

function savedRules(initial: Array<Record<string, unknown>>) {
  let records = structuredClone(initial)
  let failure = ''
  const calls: Array<{ url: string; method: string; body?: any }> = []
  const messages: Array<[string, string]> = []
  const modules: Record<string, unknown> = {
    vue,
    '@remixicon/vue': {},
    '@/lib/api': {
      async apiJson(url: string, options: { method?: string; body?: string } = {}) {
        const body = options.body ? JSON.parse(options.body) : undefined
        calls.push({ url, method: options.method || 'GET', body })
        if (options.method === 'POST' && failure) throw new Error(failure)
        if (options.method === 'DELETE') records = records.filter((record) => !url.endsWith('/' + record.id))
        const affected = records.filter((record) => body?.rule_ids?.includes(record.id))
        if (body?.operation?.action === 'set_mode') {
          for (const record of affected) record.mode = body.operation.mode
        } else if (body?.operation?.action === 'delete') {
          records = records.filter((record) => !affected.includes(record))
        }
        return { affected: structuredClone(affected), missing_ids: [] }
      },
    },
    '@/composables/usePaneRead': {
      usePaneRead: (_query: unknown, onData: (data: unknown) => void) => {
        const refresh = async () => onData({ items: structuredClone(records) })
        void refresh()
        return { loading: vue.ref(false), error: vue.ref(''), refresh }
      },
    },
    '@/stores/toasts': {
      useToastsStore: () => ({ push: (kind: string, message: string) => messages.push([kind, message]) }),
    },
    '@/i18n/settingsText': { settingsText: (key: string) => key },
    '@/stores/chat': { useChatStore: () => ({ selectedSessionId: '1' }) },
    '@/lib/lastSession': { lastSessionId: vue.ref('1') },
    '@/stores/directory': { useDirectoryStore: () => ({ currentDirectory: '' }) },
    '@/stores/directorySessionStore': { useDirectorySessionStore: () => ({ visibleDirectories: [] }) },
    '@/features/sessions/model/labels': { normalizeDirForCompare: (value: string) => value },
    '@/lib/permissionRuleScope': { permissionRuleMatchesScope },
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  const require = (id: string) => {
    if (id.endsWith('.vue')) return {}
    if (!(id in modules)) throw new Error('Unexpected component dependency: ' + id)
    return modules[id]
  }
  new Function('require', 'module', 'exports', compiled)(require, module, module.exports)
  const scope = vue.effectScope()
  scopes.push(scope)
  const panel = scope.run(() =>
    module.exports.default!.setup(vue.reactive({ scope: 'global', sessionId: undefined }), { expose() {} }),
  )!
  return {
    panel,
    calls,
    messages,
    rejectWrites(message: string) {
      failure = message
    },
  }
}

function rule(id: number, subject: string, mode = 'deny', target = '192.0.2.10:443') {
  return {
    id,
    action_key: '{}',
    subject_kind: subject,
    network_target: subject === 'network_access' ? target : null,
    tool_name: subject === 'tool' ? 'agena.shell.exec' : null,
    mode,
    scope: 'global',
    source: 'manual',
    revoked_at: null,
  }
}

describe('saved native network decisions', () => {
  test('network rows expose only native actions and reject ask before making a request', async () => {
    const { panel, calls } = savedRules([rule(1, 'network_access')])
    expect(panel.networkModeOptions.value.map((option: { value: string }) => option.value)).toEqual(['allow', 'deny'])

    await panel.setRuleMode(panel.rules.value[0], 'ask')
    expect(calls).toHaveLength(0)

    await panel.setRuleMode(panel.rules.value[0], 'allow')
    expect(calls[0]?.body.operation).toEqual({ action: 'set_mode', mode: 'allow' })
    expect(panel.rules.value[0].mode).toBe('allow')
  })

  test('a mixed selection switches to allow/deny and cannot submit an unsupported bulk mode', async () => {
    const { panel, calls } = savedRules([rule(1, 'network_access'), rule(2, 'tool', 'ask')])
    panel.selectedIds.value = new Set([1, 2])
    await vue.nextTick()

    expect(panel.bulkModeOptions.value.map((option: { value: string }) => option.value)).toEqual(['allow', 'deny'])
    expect(panel.bulkMode.value).toBe('deny')
    panel.bulkMode.value = 'auto'
    await panel.applyBulkOperation('set_mode')
    expect(calls).toHaveLength(0)

    panel.bulkMode.value = 'deny'
    await panel.applyBulkOperation('set_mode')
    expect(calls[0]?.body.rule_ids).toEqual([2, 1])
    expect(panel.rules.value.every((item: { mode: string }) => item.mode === 'deny')).toBe(true)
  })

  test('historical invalid records remain visible, surface save errors, and can be removed', async () => {
    const { panel, calls, messages, rejectWrites } = savedRules([rule(1, 'network_access', 'auto', 'legacy.example')])
    expect(panel.sortedRules.value[0].network_target).toBe('legacy.example')
    expect(panel.sortedRules.value[0].mode).toBe('auto')
    rejectWrites('network_access.host must be a literal IP')

    await panel.setRuleMode(panel.rules.value[0], 'allow')
    expect(messages.at(-1)).toEqual(['error', 'network_access.host must be a literal IP'])
    expect(panel.rules.value[0].mode).toBe('auto')

    await panel.removeRule(1)
    expect(calls.at(-1)?.method).toBe('DELETE')
    expect(panel.sortedRules.value).toHaveLength(0)
  })
})
