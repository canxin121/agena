import { describe, expect, test } from 'bun:test'
import { transcriptDiffFiles } from '../src/pages/chat/transcriptDiff'

describe('current unified diff ranges', () => {
  test('hunk ranges preserve lines that resemble file headers and empty ranges', () => {
    const [file] = transcriptDiffFiles(
      '--- a/example.txt\n+++ b/example.txt\n@@ -10,1 +20,2 @@\n--- value\n+++ value\n+next\n',
    )
    expect(file).toMatchObject({ path: 'example.txt', oldPath: 'example.txt', additions: 2, deletions: 1 })
    expect(file!.rows.slice(1)).toEqual([
      { kind: 'removed', text: '-- value', oldLine: 10, newLine: null },
      { kind: 'added', text: '++ value', oldLine: null, newLine: 20 },
      { kind: 'added', text: 'next', oldLine: null, newLine: 21 },
    ])
    expect(transcriptDiffFiles('--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n')[0]).toMatchObject({
      change: 'A',
      path: 'new.txt',
      additions: 1,
      deletions: 0,
    })
  })

  test('bare historical hunk markers do not invent line numbers', () => {
    const [file] = transcriptDiffFiles('--- a/example.txt\n+++ b/example.txt\n@@\n-old\n+new\n')
    expect(file).toMatchObject({ additions: 0, deletions: 0 })
    expect(file!.rows.every((row) => row.kind === 'note' && row.oldLine === null && row.newLine === null)).toBe(true)
  })
})
