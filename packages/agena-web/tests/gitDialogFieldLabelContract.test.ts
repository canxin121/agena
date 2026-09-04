import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

function read(name: string) {
  return readFileSync(new URL(`../src/components/git/${name}`, import.meta.url), 'utf8')
}

test('git rename and branch dialogs expose programmatic field names', () => {
  for (const name of ['GitRenameBranchDialog.vue', 'GitRenameDialog.vue']) {
    const source = read(name)
    assert.match(source, /:aria-label="t\('common\.from'\)"/)
    assert.match(source, /:aria-label="t\('common\.to'\)"/)
  }

  const branchSource = read('GitCreateBranchFromDialog.vue')
  assert.match(branchSource, /createBranchFrom\.fields\.branchName/)
  assert.match(branchSource, /createBranchFrom\.fields\.startPoint/)

  const actionSource = read('GitBranchActionDialog.vue')
  assert.match(actionSource, /:aria-label="t\('git\.ui\.branchAction\.branchNamePlaceholder'\)"/)
})

test('git compare and remotes fields keep meaningful accessible names after values replace placeholders', () => {
  const compare = read('GitCompareDialog.vue')
  assert.ok((compare.match(/:aria-label=/g) || []).length >= 4)

  const remotes = read('GitRemotesDialog.vue')
  assert.ok((remotes.match(/:aria-label=/g) || []).length >= 4)
})
