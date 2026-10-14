import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const viewSource = readFileSync(resolve(import.meta.dir, '../src/pages/chat/ChatPageView.vue'), 'utf8')
const chipSource = readFileSync(resolve(import.meta.dir, '../src/components/chat/SessionChip.vue'), 'utf8')
const sectionSources = {
  sessionWork: readFileSync(resolve(import.meta.dir, '../src/components/chat/SessionWorkSection.vue'), 'utf8'),
  plan: readFileSync(resolve(import.meta.dir, '../src/components/chat/PlanSection.vue'), 'utf8'),
  btw: readFileSync(resolve(import.meta.dir, '../src/components/chat/BtwSection.vue'), 'utf8'),
  workspaceChanges: readFileSync(
    resolve(import.meta.dir, '../src/components/chat/WorkspaceChangesSection.vue'),
    'utf8',
  ),
}

test('no session section docks above the composer any more', () => {
  // The emerald band above the composer is gone: it stole transcript height and
  // its chips overflowed the composer once they showed long summaries.
  assert.doesNotMatch(viewSource, /data-session-sections/)
  assert.doesNotMatch(viewSource, /sessionSectionsDock/)
})

test('every session section renders inside the composer footer', () => {
  const composerStart = viewSource.indexOf('<Composer')
  assert.notEqual(composerStart, -1, 'the composer must render')
  const footerStart = viewSource.indexOf('#bottomLeft', composerStart)
  assert.notEqual(footerStart, -1, 'the composer footer must own a bottom-left row')
  for (const tag of ['<PlanSection', '<WorkspaceChangesSection', '<SessionWorkSection', '<BtwSection']) {
    const at = viewSource.indexOf(tag, composerStart)
    assert.ok(at > footerStart, `${tag} must render inside the composer footer`)
  }
  const transcript = viewSource.slice(0, composerStart)
  for (const tag of ['<PlanSection', '<SessionWorkSection', '<BtwSection', '<WorkspaceChangesSection']) {
    assert.ok(!transcript.includes(tag), `${tag} must not render inside the transcript`)
  }
})

test('the footer chips stay short: an icon, a number, and a tooltip', () => {
  assert.match(chipSource, /data-session-chip/)
  assert.match(chipSource, /max-w-32 truncate/)
  assert.match(chipSource, /:title="hint \|\| title"/)
  // Footer triggers reuse the same compact, borderless treatment and spacing as model controls.
  assert.match(chipSource, /min-h-7[^\n]*gap-1 rounded px-1 py-0\.5/)
  assert.doesNotMatch(chipSource, /rounded-md border border-border\/60 bg-background/)
  for (const [name, source] of Object.entries(sectionSources)) {
    assert.match(source, /<SessionChip/, `${name} must render through the shared chip`)
    assert.doesNotMatch(source, /SessionSection/, `${name} must not use the removed docked section`)
    assert.match(source, /<template #icon>/, `${name} must render a chip icon`)
    assert.match(source, /class="h-3 w-3 shrink-0"/, `${name} must use model-sized icons`)
  }
})

test('the composer footer keeps its chips apart and straddles the bottom border', () => {
  const composerSource = readFileSync(resolve(import.meta.dir, '../src/components/chat/Composer.vue'), 'utf8')
  const at = composerSource.indexOf('$slots.bottomLeft')
  assert.notEqual(at, -1, 'the composer must declare its footer chip slot')
  const footer = composerSource.slice(at, at + 400)
  assert.match(footer, /gap-2/, 'the footer row must space adjacent chips')
  assert.ok(
    composerSource.includes('bottom-0 z-10 flex min-w-0 translate-y-1/2'),
    'chips must break the bottom border like the model row',
  )
  assert.match(composerSource, /max-w-full[^\"]*overflow-x-auto[^\"]*bg-background/)
  assert.ok(chipSource.includes('font-mono text-[11px] font-medium'), 'chip labels should match model typography')
})

test('a chip panel is portalled and bounded so the composer cannot clip it', () => {
  assert.match(chipSource, /<PopoverPortal>/)
  assert.match(chipSource, /side="top"/)
  assert.match(chipSource, /max-h-\[min\(70dvh,40rem\)\]/)
  assert.match(chipSource, /overflow-auto/)
})

test('one edited file is one row with a git-style merged line stat', () => {
  const source = sectionSources.workspaceChanges
  // A file lists once and shows +added/−removed instead of a per-operation count.
  assert.match(source, /v-for="file in files"/)
  assert.match(source, /:key="file\.path"/)
  assert.match(source, /\+\{\{ file\.added \}\}/)
  assert.match(source, /−\{\{ file\.removed \}\}/)
  assert.doesNotMatch(source, /operation_count/)
})
