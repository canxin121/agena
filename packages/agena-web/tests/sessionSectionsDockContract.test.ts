import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const viewSource = readFileSync(resolve(import.meta.dir, '../src/pages/chat/ChatPageView.vue'), 'utf8')
const sectionSources = {
  sessionWork: readFileSync(resolve(import.meta.dir, '../src/components/chat/SessionWorkSection.vue'), 'utf8'),
  plan: readFileSync(resolve(import.meta.dir, '../src/components/chat/PlanSection.vue'), 'utf8'),
  btw: readFileSync(resolve(import.meta.dir, '../src/components/chat/BtwSection.vue'), 'utf8'),
  workspaceChanges: readFileSync(
    resolve(import.meta.dir, '../src/components/chat/WorkspaceChangesSection.vue'),
    'utf8',
  ),
}

test('the plan and the background work dock above the composer instead of trailing the transcript', () => {
  // Both used to render inside the transcript scroll area, so the plan sat at
  // the end of the messages and the background panel drifted behind them. One
  // dock above the composer now owns every session section.
  assert.doesNotMatch(viewSource, /data-session-sections=/)
  const dockStart = viewSource.indexOf('data-session-sections-dock="true"')
  assert.notEqual(dockStart, -1, 'the shared session sections dock must exist')
  const composerStart = viewSource.indexOf('<Composer', dockStart)
  assert.notEqual(composerStart, -1, 'the composer must follow the dock')

  const dock = viewSource.slice(dockStart, composerStart)
  for (const tag of ['<SessionWorkSection', '<PlanSection', '<BtwSection', '<WorkspaceChangesSection']) {
    assert.ok(dock.includes(tag), `${tag} must render inside the dock above the composer`)
  }

  const transcript = viewSource.slice(0, dockStart)
  for (const tag of ['<SessionWorkSection', '<PlanSection', '<BtwSection']) {
    assert.ok(!transcript.includes(tag), `${tag} must not render inside the transcript`)
  }
})

test('the dock sits between the transcript and the composer', () => {
  const scrollStart = viewSource.indexOf('ref="scrollEl"')
  const dockStart = viewSource.indexOf('data-session-sections-dock="true"')
  const composerStart = viewSource.indexOf('<Composer')
  assert.ok(scrollStart >= 0 && dockStart > scrollStart, 'the dock must follow the transcript scroll area')
  assert.ok(composerStart > dockStart, 'the dock must sit above the composer')
})

test('every docked section bounds its own body so several sections fit above the composer', () => {
  for (const [name, source] of Object.entries(sectionSources)) {
    assert.match(source, /\bdocked\b/, `${name} must render its section in docked mode`)
  }
  for (const name of ['sessionWork', 'plan', 'btw'] as const) {
    assert.match(sectionSources[name], /max-h-\[min\(45dvh,28rem\)\]/, `${name} must bound its body height`)
    assert.match(sectionSources[name], /overflow-auto/, `${name} must scroll its own body`)
  }
})
