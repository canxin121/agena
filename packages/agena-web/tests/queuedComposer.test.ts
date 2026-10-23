import { expect, test } from 'bun:test'
import { appendQueuedComposerDocuments } from '../src/pages/chat/queuedComposer'
import type { AttachedFile } from '../src/pages/chat/useChatAttachments'
import type { ComposerSegment } from '../src/pages/chat/composerInput'

const file = (id: string): AttachedFile => ({
  id,
  filename: `${id}.pdf`,
  size: 20,
  mime: 'application/pdf',
  state: 'ready',
  serverPath: `/workspace/${id}.pdf`,
})
const resource = (id: string, stored: boolean) => ({
  type: 'activity',
  activity: {
    id,
    payload: {
      activity_type: 'resource',
      name: `${id}.pdf`,
      media_type: 'application/pdf',
      delivery: 'model_input',
      reference: stored
        ? { reference_type: 'artifact', uri: 'agena:media:v1:immutable', sha256: 'frozen-hash' }
        : { reference_type: 'workspace_path', path: `${id}.pdf` },
    },
  },
})

test('returning queued input merges with a newer draft and preserves inline attachment order', () => {
  const existing = file('existing')
  const segments: ComposerSegment[] = [
    { type: 'text', text: 'new draft ' },
    { type: 'attachment', id: 'existing' },
  ]
  const document = [{ type: 'text', text: 'before ' }, resource('restored', false), { type: 'text', text: ' after' }]
  const merged = appendQueuedComposerDocuments(segments, [existing], [document])
  expect(merged.text).toBe('new draft \n\nbefore  after')
  expect(merged.segments).toEqual([
    ...segments,
    { type: 'text', text: '\n\n' },
    { type: 'text', text: 'before ' },
    { type: 'attachment', id: 'restored' },
    { type: 'text', text: ' after' },
  ])
  expect(merged.files).toHaveLength(2)
  expect(merged.files[0]).toBe(existing)
  expect(merged.files[1]!.serverPath).toBe('restored.pdf')
  expect(segments).toHaveLength(2)
})

test('a stored media input retains its immutable resource node for explicit resubmission', () => {
  const attachment = resource('frozen', true)
  const merged = appendQueuedComposerDocuments([], [], [[attachment]])
  expect(merged.segments).toEqual([{ type: 'attachment', id: 'frozen' }])
  expect(merged.files[0]!.filename).toBe('frozen.pdf')
  expect(merged.files[0]!.composerNode).toEqual(attachment)
  expect(merged.files[0]!.serverPath).toBeUndefined()
  expect(merged.text).toBe('')
})

test('structured content queued from TUI is retained when returning it in Web', () => {
  const activity = {
    type: 'activity',
    activity: { id: 'command', payload: { activity_type: 'command_reference', name: 'review' } },
  }
  const merged = appendQueuedComposerDocuments([], [], [[{ type: 'text', text: 'run ' }, activity]])
  expect(merged.files[0]!.filename).toBe('review')
  expect(merged.files[0]!.composerNode).toEqual(activity)
  expect(merged.segments).toEqual([
    { type: 'text', text: 'run ' },
    { type: 'attachment', id: 'command' },
  ])
})
