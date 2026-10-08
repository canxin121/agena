import { expect, test } from 'bun:test'
import { readContentForCopy } from '../src/lib/contentCopy'
import type { ContentCursor, ContentPage } from '../src/lib/content'

function page(sequences: number[], last: number, dropped = 0): ContentPage {
  const cursor = { epoch: 'epoch', sequence: last }
  return { resource: { resource_id:'source', kind:'log', owner_session_id:1,part_id:2,state:'complete',cursor,committed_cursor:cursor,total_bytes:10,dropped_bytes:dropped,retained_ranges:[] },
    chunks:sequences.map((sequence) => ({cursor:{...cursor,sequence},captured_at_ms:1000,payload:{type:'log',stream:'stdout',text:`  ${sequence}\n\n`}})),next_cursor:cursor,has_more:false,gap:dropped > 0 }
}

async function copy(target: ContentCursor, value: ContentPage) {
  let body = ''
  await readContentForCopy(target, async () => value, 'missing', 'too large', async (text) => { body += text })
  return body
}

test('content copy preserves exact whitespace and stops at the chosen cursor', async () => {
  expect(await copy({epoch:'epoch',sequence:2}, page([1,2,3],3))).toBe('  1\n\n  2\n\n')
})
test('content copy marks empty loss pages and missing leading and trailing ranges', async () => {
  expect(await copy({epoch:'epoch',sequence:3}, page([],3,10))).toContain('[missing]')
  expect(await copy({epoch:'epoch',sequence:3}, page([2],3,10))).toBe('\n[missing]\n  2\n\n\n[missing]\n')
})
test('content copy rejects a generation replacement rather than joining executions', async () => {
  await expect(copy({epoch:'replaced',sequence:2}, page([1,2],2))).rejects.toThrow('generation')
})
