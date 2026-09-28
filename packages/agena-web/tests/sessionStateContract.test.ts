import assert from 'node:assert/strict'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import {
  SESSION_STATE_ATTENTION_KINDS,
  SESSION_STATE_BUSY_KINDS,
  SESSION_STATE_FAILED_KINDS,
  SESSION_STATE_KINDS,
} from '../src/generated/agenaState'
import {
  normalizeSessionState,
  sessionStateData,
  sessionStateExecution,
  sessionStateIsBusy,
  sessionStateKind,
  sessionStateNeedsAttention,
  sessionStateRequests,
} from '../src/types/chat'

// The contract between the generated mirror and the shipped parser/helpers.
// Every state string the frontend knows comes from
// `src/generated/agenaState.ts`; nothing here may invent a second vocabulary.

const packageRoot = join(dirname(fileURLToPath(import.meta.url)), '..')
const sourceDir = join(packageRoot, 'src')
const generatedModule = 'src/generated/agenaState.ts'

// Wire literals only a *session state* union or table can contain. Other UI
// vocabularies may reuse 'running' or 'failed' without duplicating a state.
const sessionOnlyKindTokens = ['awaiting_interaction', 'creating']

function sourceFiles(dir: string): string[] {
  const files: string[] = []
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) files.push(...sourceFiles(full))
    else if (/\.(ts|tsx|vue)$/.test(entry)) files.push(full)
  }
  return files
}

test('the generated kind list is the only state vocabulary the parser accepts', () => {
  assert.equal(SESSION_STATE_KINDS.length, 5)
  for (const kind of SESSION_STATE_KINDS) {
    assert.equal(normalizeSessionState({ kind, data: {} }).kind, kind)
  }
  for (const unknown of ['succeeded', 'READY', '', 'running ', 'unknown']) {
    assert.equal(normalizeSessionState({ kind: unknown, data: {} }).kind, 'ready', unknown)
  }
  assert.equal(normalizeSessionState(undefined).kind, 'ready')
  assert.equal(normalizeSessionState(null).kind, 'ready')
  assert.equal(normalizeSessionState({ kind: 'running' }).kind, 'running')
})

test('the generated classification constants match the shipped state helpers', () => {
  for (const kind of SESSION_STATE_KINDS) {
    const state = normalizeSessionState({ kind, data: {} })
    assert.equal(sessionStateKind(state), kind)
    assert.equal(sessionStateIsBusy(state), SESSION_STATE_BUSY_KINDS.includes(kind), kind)
    assert.equal(
      sessionStateNeedsAttention(state),
      SESSION_STATE_ATTENTION_KINDS.includes(kind),
      kind,
    )
  }
  assert.ok(SESSION_STATE_FAILED_KINDS.includes('failed'))
  assert.equal(sessionStateNeedsAttention(normalizeSessionState({ kind: 'failed' })), true)

  // A running session with a pending request needs attention without changing
  // its kind.
  const gated = normalizeSessionState({
    kind: 'running',
    data: { workflow: 'quiescent', requests: [{ kind: 'permission', request_id: 'req-1' }] },
  })
  assert.equal(gated.kind, 'running')
  assert.equal(sessionStateIsBusy(gated), true)
  assert.equal(sessionStateNeedsAttention(gated), true)
  assert.equal(sessionStateRequests(gated).length, 1)
})

test('one parser keeps exactly the payload fields the generated type allows', () => {
  assert.deepEqual(sessionStateData(normalizeSessionState({ kind: 'creating' })), {})

  const ready = normalizeSessionState({
    kind: 'ready',
    data: { last_failure: { code: 'boom' }, ignored: 1 },
  })
  assert.deepEqual(Object.keys(sessionStateData(ready)), ['last_failure'])

  const running = normalizeSessionState({
    kind: 'running',
    data: {
      workflow: 'tool_pending',
      execution: { execution_id: 'exec-1', phase: 'executing_tools' },
      requests: [{ kind: 'permission', request_id: 'req-1' }],
      not_a_state_field: true,
    },
  })
  assert.equal(sessionStateData(running).workflow, 'tool_pending')
  assert.equal(sessionStateExecution(running)?.phase, 'executing_tools')
  assert.equal('not_a_state_field' in sessionStateData(running), false)

  // Payloads the mirror does not know degrade to the documented default
  // instead of leaking a foreign state into the UI.
  const degraded = normalizeSessionState({
    kind: 'running',
    data: { workflow: 'invented', execution: { execution_id: 'exec-1', phase: 'invented' } },
  })
  assert.equal(sessionStateData(degraded).workflow, 'quiescent')
  assert.equal(sessionStateExecution(degraded), undefined)

  const awaiting = normalizeSessionState({
    kind: 'awaiting_interaction',
    data: { run_id: 7, execution: { execution_id: 'exec-2', phase: 'awaiting_interaction' } },
  })
  assert.equal(sessionStateData(awaiting).run_id, 7)
  assert.equal(sessionStateExecution(awaiting)?.execution_id, 'exec-2')

  const failed = normalizeSessionState({ kind: 'failed', data: { failure: { code: 'boom' } } })
  assert.deepEqual(sessionStateData(failed).failure, { code: 'boom' })
})

test('no module outside the generated mirror declares a second session-state vocabulary', () => {
  const offenders: string[] = []
  const tokens = sessionOnlyKindTokens.join('|')
  // A union member (`'interrupted' | …`) or a literal table with two state
  // kinds in one array is a hand-written copy of the generated vocabulary.
  const unionMember = new RegExp(`'(?:${tokens})'\\s*\\|\\s*(?![|=])`)
  const literalTable = new RegExp(`\\[[^\\]]*'(?:${tokens})'[^\\]]*'(?:${tokens})'[^\\]]*\\]`)

  for (const file of sourceFiles(sourceDir)) {
    const path = relative(packageRoot, file).split('\\').join('/')
    if (path === generatedModule) continue
    const source = readFileSync(file, 'utf8')
    if (unionMember.test(source)) offenders.push(`${path}: hand-written session-state union`)
    if (literalTable.test(source)) offenders.push(`${path}: hand-written session-state table`)
  }

  assert.deepEqual(offenders, [])
})
