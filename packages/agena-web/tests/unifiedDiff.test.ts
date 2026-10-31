import { describe, expect, test } from 'bun:test'
import {
  buildUnifiedDiffModel,
  buildUnifiedMonacoDiffModel,
  buildVirtualMonacoDiffModel,
  resolveInitialTopLineFromParsedDiff,
  resolveInitialTopLineFromTextPair,
  resolveInitialTopLineFromUnifiedDiff,
} from '../src/features/git/diff/unifiedDiff'
import type { GitDiffMeta } from '../src/types/git'

const singleFileDiff = [
  'diff --git a/src/app.ts b/src/app.ts',
  'index 1111111..2222222 100644',
  '--- a/src/app.ts',
  '+++ b/src/app.ts',
  '@@ -1,4 +1,5 @@',
  ' export function greet(name: string) {',
  '-  return `hello ${name}`',
  '+  const greeting = `hello ${name}`',
  '+  return greeting',
  ' }',
  '',
].join('\n')

const twoHunkDiff = [
  '--- a/src/two.ts',
  '+++ b/src/two.ts',
  '@@ -1,2 +1,2 @@',
  '-alpha',
  '+ALPHA',
  ' beta',
  '@@ -10,2 +10,2 @@',
  '-gamma',
  '+GAMMA',
  ' delta',
  '',
].join('\n')

describe('unified diff parsing', () => {
  test('parses file header, hunk metadata, and summary from a patch', () => {
    const parsed = buildUnifiedDiffModel(singleFileDiff)

    expect(parsed.fileHeader).toEqual([
      'diff --git a/src/app.ts b/src/app.ts',
      'index 1111111..2222222 100644',
      '--- a/src/app.ts',
      '+++ b/src/app.ts',
    ])
    expect(parsed.hasPatchHeader).toBe(true)
    expect(parsed.summary).toEqual({ files: 1, hunks: 1, changedLines: 3 })
    expect(parsed.hunks).toHaveLength(1)
    expect(parsed.hunks[0]).toMatchObject({
      id: '1',
      header: '@@ -1,4 +1,5 @@',
      range: '-1,4 +1,5',
      oldStart: 1,
      oldCount: 4,
      newStart: 1,
      newCount: 5,
      additions: 2,
      deletions: 1,
      anchorLine: 2,
      patchReady: true,
    })
    expect(parsed.hunks[0]!.patch.startsWith('diff --git a/src/app.ts b/src/app.ts\n')).toBe(true)
  })

  test('keeps multiple hunks independent', () => {
    const parsed = buildUnifiedDiffModel(twoHunkDiff)

    expect(
      parsed.hunks.map((hunk) => [hunk.id, hunk.range, hunk.oldStart, hunk.newStart, hunk.additions, hunk.deletions]),
    ).toEqual([
      ['1', '-1,2 +1,2', 1, 1, 1, 1],
      ['2', '-10,2 +10,2', 10, 10, 1, 1],
    ])
    expect(parsed.summary).toEqual({ files: 1, hunks: 2, changedLines: 4 })
  })

  test('normalizes CRLF patch input', () => {
    const parsed = buildUnifiedDiffModel(singleFileDiff.replace(/\n/g, '\r\n'))

    expect(parsed.hunks).toHaveLength(1)
    expect(parsed.hunks[0]!.additions).toBe(2)
    expect(parsed.hunks[0]!.lines.some((line) => line.includes('\r'))).toBe(false)
  })

  test('honors precomputed hunk metadata', () => {
    const meta: GitDiffMeta = {
      fileHeader: ['--- a/docs/notes.md', '+++ b/docs/notes.md'],
      hasPatchHeader: true,
      hunks: [
        {
          id: '1',
          header: '@@ -1,1 +1,2 @@',
          range: '-1,1 +1,2',
          oldStart: 1,
          oldCount: 1,
          newStart: 1,
          newCount: 2,
          additions: 1,
          deletions: 0,
          anchorLine: 2,
          lines: [' keep', '+added'],
          patch: '--- a/docs/notes.md\n+++ b/docs/notes.md\n@@ -1,1 +1,2 @@\n keep\n+added\n',
          patchReady: true,
        },
      ],
      summary: { files: 1, hunks: 1, changedLines: 1 },
    }

    const parsed = buildUnifiedDiffModel('', meta)

    expect(parsed.summary).toEqual({ files: 1, hunks: 1, changedLines: 1 })
    expect(parsed.hunks[0]).toMatchObject({ anchorLine: 2, patchReady: true })
    expect(parsed.hunks[0]!.patch).toBe(meta.hunks[0]!.patch)
    expect(buildUnifiedMonacoDiffModel('', meta).path).toBe('docs/notes.md')
  })
})

describe('monaco and virtual diff models', () => {
  test('rebuilds original and modified text from a patch', () => {
    const model = buildUnifiedMonacoDiffModel(singleFileDiff)

    expect(model.path).toBe('src/app.ts')
    expect(model.original.split('\n')).toEqual([
      '@@ -1,4 +1,5 @@',
      'export function greet(name: string) {',
      '  return `hello ${name}`',
      '}',
    ])
    expect(model.modified.split('\n')).toEqual([
      '@@ -1,4 +1,5 @@',
      'export function greet(name: string) {',
      '  const greeting = `hello ${name}`',
      '  return greeting',
      '}',
    ])
    expect(model.hasChanges).toBe(true)
    expect(model.initialTopLine).toBe(2)
    expect(model.originalLineNumbers).toEqual([null, 1, 2, 3])
    expect(model.modifiedLineNumbers).toEqual([null, 1, 2, 3, 4])
  })

  test('combines every file of a multi-file patch', () => {
    const multi = [
      singleFileDiff,
      'diff --git a/src/other.ts b/src/other.ts',
      '--- a/src/other.ts',
      '+++ b/src/other.ts',
      '@@ -1,1 +1,1 @@',
      '-old',
      '+new',
      '',
    ].join('\n')

    const model = buildUnifiedMonacoDiffModel(multi)

    expect(model.original).toContain('diff --git a/src/app.ts b/src/app.ts')
    expect(model.original).toContain('diff --git a/src/other.ts b/src/other.ts')
    expect(model.modified.split('\n')).toContain('new')
  })

  test('prefers patch text for a virtual diff when requested', () => {
    const model = buildVirtualMonacoDiffModel({ diff: singleFileDiff, preferPatch: true })

    expect(model.modelId).toBe('virtual:src/app.ts')
    expect(model.path).toBe('src/app.ts')
    expect(model.hasChanges).toBe(true)
    expect(model.modified.split('\n')).toContain('  return greeting')
  })

  test('uses explicit snapshots without a patch', () => {
    const model = buildVirtualMonacoDiffModel({ original: 'a\nb\nc', modified: 'a\nB\nc' })

    expect(model.path).toBe('virtual.diff')
    expect(model.modelId).toBe('virtual:virtual.diff')
    expect(model.hasChanges).toBe(true)
    expect(model.initialTopLine).toBe(2)
  })

  test('reports no changes for identical snapshots', () => {
    const model = buildVirtualMonacoDiffModel({ path: 'same.txt', original: 'same', modified: 'same' })

    expect(model.hasChanges).toBe(false)
    expect(model.initialTopLine).toBeNull()
  })
})

describe('initial line resolution', () => {
  test('uses the first parsed hunk anchor', () => {
    const parsed = buildUnifiedDiffModel(singleFileDiff)

    expect(resolveInitialTopLineFromParsedDiff(parsed)).toBe(2)
    expect(resolveInitialTopLineFromUnifiedDiff(singleFileDiff)).toBe(2)
  })

  test('returns null when nothing changed', () => {
    expect(resolveInitialTopLineFromParsedDiff(null)).toBeNull()
    expect(resolveInitialTopLineFromParsedDiff(buildUnifiedDiffModel(''))).toBeNull()
    expect(resolveInitialTopLineFromTextPair('same', 'same')).toBeNull()
    expect(resolveInitialTopLineFromTextPair('a\nb', 'a\nc')).toBe(2)
  })
})
