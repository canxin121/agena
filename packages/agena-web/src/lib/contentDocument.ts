import type { JsonRecord } from '@/pages/chat/transcriptPartPresentation'
import type { JsonValue } from '@/types/json'

/** Semantic data from the common server contract. Layout and disclosure are local. */
export type DocumentBlock = JsonRecord & { type: string; id: string }
export type ContentDocument = { blocks: DocumentBlock[] }
export type DocumentMutation =
  | { type: 'insert'; after: string | null; block: DocumentBlock }
  | { type: 'replace'; block: DocumentBlock }
  | { type: 'remove'; block_id: string }
  | { type: 'append_text'; block_id: string; text: string }
  | { type: 'append_rows'; block_id: string; rows: JsonValue[][] }
  | { type: 'append_search_results'; block_id: string; items: JsonRecord[]; total: number | null }
  | { type: 'progress'; block_id: string; phase: string; completed: number; total: number | null; unit: string | null }

const encoder = new TextEncoder()
function validId(id: string) { return typeof id === 'string' && id.length > 0 && encoder.encode(id).byteLength <= 128 && !/[\u0000-\u001f\u007f-\u009f]/u.test(id) }

export function validateDocument(document: ContentDocument): ContentDocument {
  if (!Array.isArray(document.blocks) || document.blocks.length > 128) throw new Error('Document block budget exhausted')
  const ids = new Set<string>()
  for (const block of document.blocks) {
    if (!validId(block.id) || ids.has(block.id) || !['text', 'markdown', 'log', 'diff', 'command', 'table', 'search_results', 'file_changes', 'media', 'json', 'progress', 'custom'].includes(block.type)) throw new Error('Invalid document block identity')
    ids.add(block.id)
    if (block.type === 'table') {
      const columns = block.columns as JsonValue[]
      const rows = block.rows as JsonValue[][]
      if (!Array.isArray(columns) || !columns.length || columns.length > 64 || !Array.isArray(rows) || rows.some((row) => !Array.isArray(row) || row.length !== columns.length)) throw new Error('Invalid document table')
    }
    if (block.type === 'progress' && (!Number.isSafeInteger(block.completed) || Number(block.completed) < 0 || (block.total !== null && (!Number.isSafeInteger(block.total) || Number(block.total) < Number(block.completed))))) throw new Error('Invalid progress counts')
  }
  if (encoder.encode(JSON.stringify(document)).byteLength > 64 * 1024) throw new Error('Document byte budget exhausted')
  return document
}

/** Copy semantic values, preserving source whitespace rather than DOM layout. */
export function contentDocumentText(document: ContentDocument): string {
  return document.blocks.map((block) => {
    switch (block.type) {
      case 'text': case 'markdown': case 'log': return String(block.text || '')
      case 'diff': return String(block.diff || '')
      case 'json': return JSON.stringify(block.value, null, 2)
      case 'progress': return `${block.phase}: ${block.completed}${block.total === null ? '' : `/${block.total}`}${block.unit ? ` ${block.unit}` : ''}`
      case 'table': return [(block.columns as JsonValue[]).map(String).join('\t'), ...(block.rows as JsonValue[][]).map((row) => row.map((value) => typeof value === 'string' ? value : JSON.stringify(value)).join('\t'))].join('\n')
      case 'search_results': return (block.items as JsonRecord[]).map((item) => [item.title, item.url, item.snippet].filter((value) => value !== null && value !== undefined).join('\n')).join('\n\n')
      case 'command': return [`$ ${block.command}`, block.stdout, block.stderr, block.exit_code === null ? '' : `exit: ${block.exit_code}`].filter((value) => value !== null && value !== undefined && value !== '').join('\n')
      case 'media': return JSON.stringify(block.artifact)
      case 'custom': return JSON.stringify(block.schema, null, 2)
      case 'file_changes': return JSON.stringify(block.changes, null, 2)
      default: return ''
    }
  }).filter((text) => text !== '').join('\n\n')
}

/** Reuse unaffected blocks. A rejected update leaves the prior state intact. */
export function updateDocument(previous: ContentDocument, event: DocumentMutation): ContentDocument {
  const blocks = previous.blocks.slice()
  const index = (id: string) => {
    if (!validId(id)) throw new Error('Invalid document block identity')
    const position = blocks.findIndex((block) => block.id === id)
    if (position < 0) throw new Error('Document mutation has no checkpoint for its block')
    return position
  }
  switch (event.type) {
    case 'insert': {
      if (blocks.some((block) => block.id === event.block.id)) throw new Error('Duplicate document block identity')
      blocks.splice(event.after === null ? 0 : index(event.after) + 1, 0, event.block)
      break
    }
    case 'replace': blocks[index(event.block.id)] = event.block; break
    case 'remove': blocks.splice(index(event.block_id), 1); break
    case 'append_text': {
      const position = index(event.block_id)
      const block = blocks[position]!
      if (!['text', 'markdown', 'log', 'diff'].includes(block.type)) throw new Error('Text requires an appendable block')
      const field = block.type === 'diff' ? 'diff' : 'text'
      blocks[position] = { ...block, [field]: String(block[field] || '') + event.text }
      break
    }
    case 'append_rows': {
      const position = index(event.block_id)
      const block = blocks[position]!
      if (block.type !== 'table') throw new Error('Rows require a table block')
      blocks[position] = { ...block, rows: [...block.rows as JsonValue[][], ...event.rows] }
      break
    }
    case 'append_search_results': {
      const position = index(event.block_id)
      const block = blocks[position]!
      if (block.type !== 'search_results') throw new Error('Items require a search-results block')
      blocks[position] = { ...block, items: [...block.items as JsonRecord[], ...event.items], total: event.total ?? block.total ?? null }
      break
    }
    case 'progress': {
      if (!validId(event.block_id)) throw new Error('Invalid progress identity')
      const position = blocks.findIndex((block) => block.id === event.block_id)
      const block: DocumentBlock = { type: 'progress', id: event.block_id, phase: event.phase, completed: event.completed, total: event.total, unit: event.unit }
      if (position < 0) blocks.push(block)
      else {
        if (blocks[position]?.type !== 'progress') throw new Error('Progress cannot replace a different block kind')
        blocks[position] = block
      }
      break
    }
  }
  return validateDocument({ blocks })
}
