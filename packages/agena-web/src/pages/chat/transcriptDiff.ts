export type DiffRow = {
  kind: 'context' | 'added' | 'removed' | 'hunk' | 'note'
  text: string
  oldLine: number | null
  newLine: number | null
}
export type DiffFile = {
  change: 'A' | 'M' | 'D' | 'R'
  path: string
  oldPath: string
  additions: number
  deletions: number
  rows: DiffRow[]
}

/** Read display hunks without treating source lines such as `+++ value` as headers. */
export function transcriptDiffFiles(diff: string): DiffFile[] {
  const files: DiffFile[] = []
  let file: DiffFile = { change: 'M', path: '', oldPath: '', additions: 0, deletions: 0, rows: [] }
  let oldLine = 1
  let newLine = 1
  let oldRemaining = 0
  let newRemaining = 0
  let legacyHunk = false
  let headerComplete = false
  const flush = () => {
    if (file.change === 'M' && file.oldPath && file.path && file.oldPath !== file.path) file.change = 'R'
    if (file.path || file.oldPath || file.rows.length) files.push(file)
    file = { change: 'M', path: '', oldPath: '', additions: 0, deletions: 0, rows: [] }
    oldRemaining = newRemaining = 0
    legacyHunk = false
    headerComplete = false
  }
  const path = (value: string) => value.replace(/\t.*$/, '').replace(/^[ab]\//, '')
  for (const raw of diff.replace(/\r\n/g, '\n').split('\n')) {
    if (raw.startsWith('diff --git ')) {
      flush()
      file.path = path(raw.slice(raw.lastIndexOf(' b/') + 1))
      continue
    }
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(raw)
    if (hunk || raw === '@@') {
      oldLine = hunk ? Number(hunk[1]) : 1
      newLine = hunk ? Number(hunk[3]) : 1
      oldRemaining = hunk ? Number(hunk[2] ?? 1) : 0
      newRemaining = hunk ? Number(hunk[4] ?? 1) : 0
      legacyHunk = !hunk
      file.rows.push({ kind: 'hunk', text: raw, oldLine: null, newLine: null })
      continue
    }
    const inHunk = legacyHunk || oldRemaining > 0 || newRemaining > 0
    if (!inHunk && raw.startsWith('--- ')) {
      if (file.rows.length || headerComplete) flush()
      const value = path(raw.slice(4))
      file.oldPath = value === '/dev/null' ? '' : value
      if (value === '/dev/null') file.change = 'A'
      continue
    }
    if (!inHunk && raw.startsWith('+++ ')) {
      headerComplete = true
      const value = path(raw.slice(4))
      file.path = value === '/dev/null' ? file.oldPath : value
      if (value === '/dev/null') file.change = 'D'
      continue
    }
    if (raw.startsWith('new file mode ')) {
      file.change = 'A'
      continue
    }
    if (raw.startsWith('deleted file mode ')) {
      file.change = 'D'
      continue
    }
    if (raw.startsWith('rename from ')) {
      file.oldPath = raw.slice(12)
      continue
    }
    if (raw.startsWith('rename to ')) {
      file.path = raw.slice(10)
      continue
    }
    if (inHunk && /^[ +\-]/.test(raw)) {
      const kind = raw[0] === '+' ? 'added' : raw[0] === '-' ? 'removed' : 'context'
      file.rows.push({
        kind,
        text: raw.slice(1),
        oldLine: kind === 'added' ? null : oldLine++,
        newLine: kind === 'removed' ? null : newLine++,
      })
      if (kind !== 'added') oldRemaining--
      if (kind !== 'removed') newRemaining--
      if (kind === 'added') file.additions++
      if (kind === 'removed') file.deletions++
      continue
    }
    if (raw && !/^(index |new file mode |deleted file mode |similarity index )/.test(raw)) {
      file.rows.push({ kind: 'note', text: raw, oldLine: null, newLine: null })
    }
  }
  flush()
  return files
}
