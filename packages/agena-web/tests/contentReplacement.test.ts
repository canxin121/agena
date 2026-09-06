import assert from 'node:assert/strict'
import test from 'node:test'

import { ApiError } from '../src/lib/api'
import { replaceFileContent, type FsContentSearchFileResult } from '../src/features/files/api/filesApi'
import {
  contentReplacementProgress,
  contentSearchRevisions,
  refreshContentReplacement,
} from '../src/features/files/contentReplacement'

const revision = 'a'.repeat(64)
const file: FsContentSearchFileResult = {
  path: '/workspace/a.txt',
  relativePath: 'a.txt',
  revision,
  matchCount: 1,
  matches: [],
}
const completed = {
  root: '/workspace',
  fileCount: 1,
  replacementCount: 2,
  skipped: 0,
  truncated: false,
  files: [{ path: '/workspace/a.txt', relativePath: 'a.txt', replacements: 2 }],
}

test('content replacement API sends revisions and retains partial failure details', async () => {
  const previousFetch = globalThis.fetch
  let posted: Record<string, unknown> | undefined
  globalThis.fetch = (async (_url, init) => {
    posted = JSON.parse(String(init?.body))
    return new Response(
      JSON.stringify({ error: 'File changed', details: { completed, failedPath: '/workspace/b.txt', remaining: 1 } }),
      { status: 409, headers: { 'content-type': 'application/json' } },
    )
  }) as typeof fetch
  try {
    await assert.rejects(
      replaceFileContent({
        directory: '/workspace',
        query: 'old',
        replace: 'new',
        paths: [file.path],
        expectedRevisions: contentSearchRevisions([file])!,
      }),
      (error: unknown) => {
        assert.ok(error instanceof ApiError)
        assert.equal(error.status, 409)
        assert.deepEqual(contentReplacementProgress(error), { completed, failedPath: '/workspace/b.txt', remaining: 1 })
        return true
      },
    )
    assert.deepEqual(posted?.expectedRevisions, { [file.path]: revision })
  } finally {
    globalThis.fetch = previousFetch
  }
})

test('malformed progress cannot be presented as completed work', () => {
  for (const details of [
    null,
    { completed: { ...completed, fileCount: 2 }, failedPath: 'b', remaining: 0 },
    { completed: { ...completed, replacementCount: 20 }, failedPath: 'b', remaining: 0 },
    { completed, failedPath: 'b', remaining: -1 },
    { completed: { ...completed, files: [{}] }, failedPath: 'b', remaining: 0 },
  ]) {
    const error = new ApiError('failed', 500)
    error.bodyJson = JSON.parse(JSON.stringify({ details }))
    assert.equal(contentReplacementProgress(error), null)
  }
  assert.equal(contentReplacementProgress(new Error('network failed')), null)
  assert.equal(contentSearchRevisions([{ ...file, revision: '' }]), null)
  assert.equal(contentSearchRevisions([{ ...file, revision: 'z'.repeat(64) }]), null)
})

test('uncertain completion refreshes the file and search after invalidating cached reads', async () => {
  const calls: string[] = []
  await refreshContentReplacement({
    directory: '/workspace',
    current: () => ({ directory: '/workspace', path: '/workspace/a.txt', dirty: false }),
    normalizePath: (path) => path,
    invalidate: (directory) => calls.push(`invalidate:${directory}`),
    refreshFile: async () => {
      calls.push('file')
    },
    refreshSearch: async () => {
      calls.push('search')
    },
  })
  assert.deepEqual(calls, ['invalidate:/workspace', 'file', 'search'])
})

test('replacement completion preserves a new draft and does not reload unrelated files', async () => {
  for (const dirty of [false, true]) {
    const calls: string[] = []
    await refreshContentReplacement({
      directory: '/workspace',
      changedPaths: dirty ? undefined : ['/workspace/b.txt'],
      current: () => ({ directory: '/workspace', path: '/workspace/a.txt', dirty }),
      normalizePath: (path) => path,
      invalidate: () => calls.push('invalidate'),
      refreshFile: async () => {
        calls.push('file')
      },
      refreshSearch: async () => {
        calls.push('search')
      },
    })
    assert.deepEqual(calls, ['invalidate', 'search'])
  }
})

test('switching workspaces prevents stale completion from refreshing the new workspace', async () => {
  for (const switchedBeforeRefresh of [false, true]) {
    let directory = switchedBeforeRefresh ? '/other' : '/workspace'
    const calls: string[] = []
    await refreshContentReplacement({
      directory: '/workspace',
      current: () => ({ directory, path: '/workspace/a.txt', dirty: false }),
      normalizePath: (path) => path,
      invalidate: () => calls.push('invalidate'),
      refreshFile: async () => {
        calls.push('file')
        directory = '/other'
      },
      refreshSearch: async () => {
        calls.push('search')
      },
    })
    assert.deepEqual(calls, switchedBeforeRefresh ? ['invalidate'] : ['invalidate', 'file'])
  }
})
