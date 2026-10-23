import type { JsonValue } from '@/types/json'
import type { AttachedFile } from './useChatAttachments'
import type { ComposerSegment } from './composerInput'
import { restoredComposerFiles } from './queuedComposer'

export function queuedAttachmentIsResource(file: AttachedFile): boolean {
  const node = file.composerNode
  if (!node) return true
  if (typeof node !== 'object' || Array.isArray(node)) return false
  const activity = node.activity
  if (!activity || typeof activity !== 'object' || Array.isArray(activity)) return false
  const payload = activity.payload
  return Boolean(
    payload && typeof payload === 'object' && !Array.isArray(payload) && payload.activity_type === 'resource',
  )
}

/** The modal owns its document; the conversation composer is never involved. */
export function queuedEditorState(document: JsonValue) {
  const files = restoredComposerFiles(document)
  let index = 0
  if (Array.isArray(document))
    for (const node of document)
      if (node && typeof node === 'object' && !Array.isArray(node) && node.type === 'activity') {
        const file = files[index++]!
        file.composerNode = node
        // Preview labels are shortened; editing must retain the full name.
        const activity = node.activity
        if (activity && typeof activity === 'object' && !Array.isArray(activity)) {
          const payload = activity.payload
          if (
            payload &&
            typeof payload === 'object' &&
            !Array.isArray(payload) &&
            payload.activity_type === 'resource' &&
            typeof payload.name === 'string'
          )
            file.filename = payload.name
        }
      }
  const segments: ComposerSegment[] = []
  index = 0
  if (Array.isArray(document))
    for (const node of document) {
      if (!node || typeof node !== 'object' || Array.isArray(node)) continue
      if (node.type === 'text' && typeof node.text === 'string') segments.push({ type: 'text', text: node.text })
      else if (node.type === 'activity') segments.push({ type: 'attachment', id: files[index++]!.id })
    }
  return { files, segments, text: segments.flatMap((part) => (part.type === 'text' ? [part.text] : [])).join('') }
}

export async function queuedEditorDocument(
  segments: ComposerSegment[],
  files: AttachedFile[],
  prepare: (file: AttachedFile) => Promise<JsonValue>,
): Promise<JsonValue[]> {
  const nodes = new Map<string, JsonValue>()
  for (const file of files) {
    let node = file.composerNode ?? (await prepare(file))
    if (node && typeof node === 'object' && !Array.isArray(node) && node.type === 'activity') {
      const activity = node.activity
      if (activity && typeof activity === 'object' && !Array.isArray(activity)) {
        const payload = activity.payload
        if (payload && typeof payload === 'object' && !Array.isArray(payload) && payload.activity_type === 'resource')
          node = {
            ...node,
            activity: {
              ...activity,
              payload: { ...payload, name: file.filename, delivery: file.delivery || 'reference' },
            },
          }
      }
    }
    nodes.set(file.id, node)
  }
  const document: JsonValue[] = []
  const used = new Set<string>()
  for (const part of segments) {
    if (part.type === 'text') {
      if (part.text) document.push({ type: 'text', text: part.text })
    } else if (nodes.has(part.id) && !used.has(part.id)) {
      document.push(nodes.get(part.id)!)
      used.add(part.id)
    }
  }
  for (const [id, node] of nodes) if (!used.has(id)) document.push(node)
  return document
}
