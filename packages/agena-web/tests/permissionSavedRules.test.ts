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

function savedRules(
  initial: Array<Record<string, unknown>>,
  options: {
    scope?: 'global' | 'workspace' | 'session' | 'effective'
    sessionId?: string
    workspaceId?: number
    compact?: boolean
    sessions?: Record<string, { workspace_id?: number | null }>
  } = {},
) {
  let records = structuredClone(initial)
  let failure = ''
  const calls: Array<{ url: string; method: string; body?: any }> = []
  const messages: Array<[string, string]> = []
  const sessionReads: string[] = []
  const sessionGates = new Map<string, Promise<void>>()
  const sessionFailures = new Map<string, string>()
  const sessionMetadata = vue.reactive<Record<string, { id: string; workspace_id?: number | null }>>({
    '1': { id: '1', workspace_id: options.workspaceId },
  })
  let writeGate: Promise<void> | undefined
  const modules: Record<string, unknown> = {
    vue,
    '@remixicon/vue': {},
    '@/lib/api': {
      async apiJson(url: string, options: { method?: string; body?: string } = {}) {
        const body = options.body ? JSON.parse(options.body) : undefined
        calls.push({ url, method: options.method || 'GET', body })
        if (writeGate && (options.method === 'POST' || options.method === 'DELETE')) await writeGate
        if (options.method === 'POST' && failure) throw new Error(failure)
        if (url === '/api/v1/permission-rules' && options.method === 'POST') {
          const created = {
            id: Math.max(0, ...records.map((record) => Number(record.id))) + 1,
            ...body,
            source: 'manual',
            revoked_at: null,
          }
          records.push(created)
          return structuredClone(created)
        }
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
    '@/stores/chat': {
      useChatStore: () => ({
        selectedSessionId: '1',
        selectedSession: { workspace_id: options.workspaceId },
        getSessionById: (id: string) => sessionMetadata[id] || null,
        cacheSessionMetadata(entries: Array<{ id: string; workspace_id?: number | null }>) {
          for (const entry of entries) sessionMetadata[entry.id] = structuredClone(entry)
        },
      }),
    },
    '@/stores/chat/api': {
      async getSession(id: string) {
        sessionReads.push(id)
        const session = { id, ...(options.sessions?.[id] || { workspace_id: id === '1' ? options.workspaceId : null }) }
        await sessionGates.get(id)
        const message = sessionFailures.get(id)
        if (message) throw new Error(message)
        return session
      },
    },
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
  const props = vue.reactive({
    scope: options.scope || 'global',
    sessionId: options.sessionId,
    compact: options.compact,
  })
  const panel = scope.run(() => module.exports.default!.setup(props, { expose() {} }))!
  return {
    panel,
    props,
    calls,
    messages,
    sessionReads,
    sessionMetadata,
    holdSessionReads(id: string) {
      let release!: () => void
      sessionGates.set(
        id,
        new Promise<void>((resolve) => {
          release = resolve
        }),
      )
      return () => {
        sessionGates.delete(id)
        release()
      }
    },
    rejectSessionRead(id: string, message: string) {
      sessionFailures.set(id, message)
    },
    holdWrites() {
      let release!: () => void
      writeGate = new Promise<void>((resolve) => {
        release = resolve
      })
      return () => {
        writeGate = undefined
        release()
      }
    },
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

describe('compact saved approval rules', () => {
  test('creation opens on demand and persists independently in the selected scope', async () => {
    const { panel, calls } = savedRules([], { compact: true })
    expect(panel.createExpanded.value).toBe(false)
    await panel.toggleCreateForm()
    expect(panel.createExpanded.value).toBe(true)
    expect(calls).toHaveLength(0)

    panel.newToolName.value = ' agena.shell.exec '
    panel.newQualifier.value = ' git status '
    panel.newMode.value = 'ask'
    await panel.createRule()

    expect(calls).toEqual([
      {
        url: '/api/v1/permission-rules',
        method: 'POST',
        body: {
          subject_kind: 'tool',
          tool_name: 'agena.shell.exec',
          qualifier: 'git status',
          mode: 'ask',
          scope: 'global',
        },
      },
    ])
    expect(panel.sortedRules.value).toHaveLength(1)
    expect(panel.createExpanded.value).toBe(false)
    expect(panel.newToolName.value).toBe('')
  })

  test('a rejected creation keeps the expanded form and its values available for retry', async () => {
    const { panel, calls, rejectWrites } = savedRules([], { compact: true })
    await panel.toggleCreateForm()
    panel.newToolName.value = 'agena.shell.exec'
    panel.newQualifier.value = 'git status'
    rejectWrites('permission rule could not be saved')
    await panel.createRule()
    expect(panel.createExpanded.value).toBe(true)
    expect(panel.newToolName.value).toBe('agena.shell.exec')
    expect(panel.newQualifier.value).toBe('git status')
    expect(panel.createError.value).toBe('permission rule could not be saved')
    expect(panel.createBusy.value).toBe(false)

    rejectWrites('')
    await panel.createRule()
    expect(calls.at(-1)?.body).toEqual(calls[0]?.body)
    expect(panel.createExpanded.value).toBe(false)
  })

  test.each(['session', 'effective'] as const)('%s exposes saved rules without enabling writes', async (scope) => {
    const currentRule = { ...rule(1, 'tool', 'ask'), scope: 'session', session_id: 1, workspace_id: 7 }
    const { panel, calls } = savedRules([currentRule], { compact: true, scope, sessionId: '1', workspaceId: 7 })
    expect(panel.filteredRules.value).toHaveLength(1)
    expect(panel.canEdit.value).toBe(false)

    await panel.toggleCreateForm()
    panel.newToolName.value = 'agena.shell.exec'
    await panel.createRule()
    await panel.setRuleMode(panel.rules.value[0], 'allow')
    await panel.removeRule(1)
    panel.selectedIds.value = new Set([1])
    await panel.applyBulkOperation('delete')
    expect(panel.createExpanded.value).toBe(false)
    expect(calls).toHaveLength(0)
    expect(panel.filteredRules.value).toHaveLength(1)
  })

  test('search and state filtering retain the visible rule and bulk selection semantics', async () => {
    const { panel } = savedRules(
      [
        rule(1, 'tool', 'ask'),
        { ...rule(2, 'tool'), tool_name: 'agena.fs.read' },
        { ...rule(3, 'tool'), revoked_at: '2026-01-01T00:00:00Z' },
      ],
      { compact: true },
    )
    panel.query.value = 'agena.shell.exec'
    panel.stateFilter.value = 'active'
    await vue.nextTick()
    expect(panel.filteredRules.value.map((item: { id: number }) => item.id)).toEqual([1])
    panel.toggleAllSelection()
    expect([...panel.selectedIds.value]).toEqual([1])

    panel.stateFilter.value = 'revoked'
    await vue.nextTick()
    expect(panel.filteredRules.value.map((item: { id: number }) => item.id)).toEqual([3])
    expect(panel.selectedIds.value.size).toBe(0)
    panel.clearFilters()
    expect(panel.filteredRules.value).toHaveLength(3)
  })

  test('effective rules resolve the target workspace without borrowing the active chat workspace', async () => {
    const fixture = savedRules(
      [
        rule(1, 'tool'),
        { ...rule(2, 'tool'), scope: 'workspace', workspace_id: 7 },
        { ...rule(3, 'tool'), scope: 'workspace', workspace_id: 22 },
        { ...rule(4, 'tool'), scope: 'session', session_id: 1 },
        { ...rule(5, 'tool'), scope: 'session', session_id: 2 },
      ],
      { scope: 'effective', sessionId: '2', workspaceId: 7, sessions: { '2': { workspace_id: 22 } } },
    )
    expect(fixture.panel.activeWorkspaceId.value).toBeNull()
    expect(fixture.panel.sortedRules.value.map((item: { id: number }) => item.id)).toEqual([5, 1])
    await vue.nextTick()
    await vue.nextTick()
    expect(fixture.panel.activeWorkspaceId.value).toBe(22)
    expect(fixture.panel.sortedRules.value.map((item: { id: number }) => item.id)).toEqual([5, 3, 1])
    expect(fixture.sessionReads).toEqual(['2'])
  })

  test('a late target metadata response cannot replace the newly selected workspace', async () => {
    const fixture = savedRules(
      [
        { ...rule(1, 'tool'), scope: 'workspace', workspace_id: 22 },
        { ...rule(2, 'tool'), scope: 'workspace', workspace_id: 33 },
      ],
      {
        scope: 'effective',
        sessionId: '1',
        workspaceId: 7,
        sessions: { '2': { workspace_id: 22 }, '3': { workspace_id: 33 } },
      },
    )
    await vue.nextTick()
    const release = fixture.holdSessionReads('2')
    fixture.props.sessionId = '2'
    await vue.nextTick()
    expect(fixture.panel.activeWorkspaceId.value).toBeNull()
    fixture.props.sessionId = '3'
    await vue.nextTick()
    await vue.nextTick()
    await vue.nextTick()
    expect(fixture.panel.activeWorkspaceId.value).toBe(33)
    release()
    await vue.nextTick()
    await vue.nextTick()
    expect(fixture.panel.activeWorkspaceId.value).toBe(33)
    expect(fixture.panel.sortedRules.value.map((item: { id: number }) => item.id)).toEqual([2])
    expect(fixture.sessionMetadata['2']).toBeUndefined()
  })

  test('unknown target metadata leaves workspace rules unavailable and reports the read error', async () => {
    const fixture = savedRules([{ ...rule(1, 'tool'), scope: 'workspace', workspace_id: 7 }], {
      scope: 'effective',
      sessionId: '1',
      workspaceId: 7,
    })
    fixture.rejectSessionRead('2', 'Selected conversation could not be loaded')
    fixture.props.sessionId = '2'
    await vue.nextTick()
    await vue.nextTick()
    await vue.nextTick()
    expect(fixture.panel.activeWorkspaceId.value).toBeNull()
    expect(fixture.panel.sortedRules.value).toEqual([])
    expect(fixture.panel.targetWorkspaceError.value).toBe('Selected conversation could not be loaded')
  })

  test('pending remembered-rule mutations expose busy until persistence finishes', async () => {
    const fixture = savedRules([], { compact: true })
    fixture.panel.newToolName.value = 'agena.shell.exec'
    const release = fixture.holdWrites()
    const saving = fixture.panel.createRule()
    expect(fixture.panel.busy.value).toBe(true)
    release()
    await saving
    expect(fixture.panel.busy.value).toBe(false)
    expect(fixture.panel.sortedRules.value).toHaveLength(1)
  })
})
