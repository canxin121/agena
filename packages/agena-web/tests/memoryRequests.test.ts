import { describe, expect, test } from 'bun:test'
import type { AssistantMemory, MemoryKind } from '../src/generated/agenaState'
import {
  assistantMemoryRequest,
  isMemoryKind,
  memoryBatchDeleteRequest,
  memoryRevision,
  memoryWriteRequest,
} from '../src/lib/memoryRequests'

const sha256 = 'ab'.repeat(32)

function memory(kind: MemoryKind = 'procedure'): AssistantMemory {
  return {
    name: 'database-recovery',
    description: 'Recovery steps',
    body: 'Restart the pool after fixing the connection limit.',
    sha256,
    truncated: false,
    metadata: {
      id: 'mem_database-recovery',
      revision: 4,
      kind,
      scope: { kind: 'workspace' },
      status: 'active',
      origin: 'explicit',
      importance: 50,
      created_at_ms: 1000,
      updated_at_ms: 2000,
      provenance_verified: true,
      source_suppressed: false,
      supersedes: [],
    },
  }
}

describe('current memory mutation contracts', () => {
  test('edits carry the observed revision and required kind', () => {
    expect(memoryWriteRequest('preference', 'Use Chinese.', 'Language preference', sha256)).toEqual({
      kind: 'preference',
      body: 'Use Chinese.',
      description: 'Language preference',
      expected_sha256: sha256,
    })
    for (const invalid of ['', 'missing', 'aa'.repeat(31)]) {
      expect(() => memoryRevision(invalid)).toThrow()
      expect(() => memoryWriteRequest('fact', 'new body', '', invalid)).toThrow()
    }
  })

  test('assistant edits preserve the original metadata kind and deletes omit body', () => {
    const original = memory()
    expect(assistantMemoryRequest(original, { body: 'Corrected steps.', description: 'Updated' })).toEqual({
      name: original.name,
      kind: 'procedure',
      body: 'Corrected steps.',
      description: 'Updated',
      expected_sha256: sha256,
    })
    expect(assistantMemoryRequest(original, null)).toEqual({
      name: original.name,
      expected_sha256: sha256,
    })
    expect(() => assistantMemoryRequest({ ...original, sha256: '' }, null)).toThrow()
  })

  test('batch deletion requires a revision for every unique selected record', () => {
    const selected = [
      { name: 'first', sha256 },
      { name: 'second', sha256: 'cd'.repeat(32) },
    ]
    expect(memoryBatchDeleteRequest(selected)).toEqual({ records: selected })
    expect(() => memoryBatchDeleteRequest([...selected, { name: 'third', sha256: '' }])).toThrow()
    expect(() => memoryBatchDeleteRequest([selected[0]!, selected[0]!])).toThrow()
    expect(selected[0]!.sha256).toBe(sha256)
  })

  test('category selection accepts exactly the current MemoryKind values', () => {
    for (const kind of ['fact', 'preference', 'episode', 'procedure', 'task']) expect(isMemoryKind(kind)).toBe(true)
    for (const kind of ['user', 'feedback', 'project', 'reference', 'other', '']) expect(isMemoryKind(kind)).toBe(false)
  })
})
