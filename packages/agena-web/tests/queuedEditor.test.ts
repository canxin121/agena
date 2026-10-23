import { expect, test } from 'bun:test'
import { queuedAttachmentIsResource, queuedEditorDocument, queuedEditorState } from '../src/pages/chat/queuedEditor'
import type { AttachedFile } from '../src/pages/chat/useChatAttachments'
import type { JsonValue } from '../src/types/json'

const resource = (id: string, stored = true) => ({
  type: 'activity',
  activity: {
    id,
    provenance: { content_hash: 'frozen-hash' },
    payload: {
      activity_type: 'resource',
      kind: 'pdf',
      name: `${id}.pdf`,
      media_type: 'application/pdf',
      delivery: 'model_input',
      reference: stored
        ? { reference_type: 'artifact', uri: 'agena:media:v1:immutable', sha256: 'frozen-hash' }
        : { reference_type: 'workspace_path', path: `${id}.pdf` },
    },
  },
})
const noUpload = async (): Promise<JsonValue> => {
  throw new Error('existing resources must not be uploaded again')
}

test('the queue editor round trips inline text, frozen media, workspace references and opaque TUI activities', async () => {
  const command = {
    type: 'activity',
    activity: { id: 'command', payload: { activity_type: 'command_reference', name: 'review' } },
  }
  const document = [
    { type: 'text', text: 'before ' },
    resource('frozen'),
    { type: 'text', text: ' after ' },
    resource('workspace', false),
    command,
  ]
  const original = structuredClone(document)
  const state = queuedEditorState(document)
  expect(state.segments).toEqual([
    { type: 'text', text: 'before ' },
    { type: 'attachment', id: 'frozen' },
    { type: 'text', text: ' after ' },
    { type: 'attachment', id: 'workspace' },
    { type: 'attachment', id: 'command' },
  ])
  expect(state.files[0]!.filename).toBe('frozen.pdf')
  expect(queuedAttachmentIsResource(state.files[0]!)).toBe(true)
  expect(queuedAttachmentIsResource(state.files[2]!)).toBe(false)
  expect(await queuedEditorDocument(state.segments, state.files, noUpload)).toEqual(document)
  expect(document).toEqual(original)
})

test('renaming and changing delivery preserves original bytes, provenance and inline position', async () => {
  const original = resource('frozen')
  const state = queuedEditorState([{ type: 'text', text: 'inspect ' }, original, { type: 'text', text: ' here' }])
  state.files[0]!.filename = 'renamed.pdf'
  state.files[0]!.delivery = 'reference'
  const result = await queuedEditorDocument(state.segments, state.files, noUpload)
  expect(result).toEqual([
    { type: 'text', text: 'inspect ' },
    {
      ...original,
      activity: {
        ...original.activity,
        payload: { ...original.activity.payload, name: 'renamed.pdf', delivery: 'reference' },
      },
    },
    { type: 'text', text: ' here' },
  ])
  expect(original.activity.payload.name).toBe('frozen.pdf')
})

test('attachments can be removed, replaced and added without changing the text order', async () => {
  const state = queuedEditorState([
    { type: 'text', text: 'left ' },
    resource('remove'),
    resource('replace'),
    { type: 'text', text: ' right' },
  ])
  const fresh: AttachedFile = {
    id: 'fresh',
    filename: 'new.pdf',
    mime: 'application/pdf',
    size: 20,
    state: 'ready',
    delivery: 'model_input',
  }
  const added: AttachedFile = { ...fresh, id: 'added', filename: 'added.pdf' }
  const segments = state.segments.map((part) =>
    part.type === 'attachment' && part.id === 'replace' ? { ...part, id: 'fresh' } : part,
  )
  const prepared: string[] = []
  const document = await queuedEditorDocument(segments, [fresh, added], async (file) => {
    prepared.push(file.id)
    return resource(file.id)
  })
  expect(document).toEqual([
    { type: 'text', text: 'left ' },
    {
      ...resource('fresh'),
      activity: { ...resource('fresh').activity, payload: { ...resource('fresh').activity.payload, name: 'new.pdf' } },
    },
    { type: 'text', text: ' right' },
    resource('added'),
  ])
  expect(prepared).toEqual(['fresh', 'added'])
})

test('duplicate inline chips publish one attachment and failed preparation does not alter the original message', async () => {
  const document = [resource('same')]
  const state = queuedEditorState(document)
  const segments = [
    { type: 'attachment' as const, id: 'same' },
    { type: 'attachment' as const, id: 'same' },
  ]
  expect(await queuedEditorDocument(segments, state.files, noUpload)).toEqual(document)
  const extra: AttachedFile = { id: 'missing', filename: 'missing.png', mime: 'image/png', size: 1, state: 'ready' }
  await expect(queuedEditorDocument(state.segments, [...state.files, extra], noUpload)).rejects.toThrow()
  expect(state.files).toHaveLength(1)
  expect(document).toEqual([resource('same')])
})

test('editing preserves long attachment names instead of saving the shortened preview label', async () => {
  const node = resource('long')
  node.activity.payload.name = `${'long-name-'.repeat(20)}.pdf`
  const state = queuedEditorState([node])
  expect(state.files[0]!.filename).toBe(node.activity.payload.name)
  expect(await queuedEditorDocument(state.segments, state.files, noUpload)).toEqual([node])
})
