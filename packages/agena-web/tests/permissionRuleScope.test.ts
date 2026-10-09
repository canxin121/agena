import assert from 'node:assert/strict'
import test from 'node:test'

import { permissionRuleMatchesScope } from '../src/lib/permissionRuleScope'

test('workspace permission rows require an exact, resolved workspace identity', () => {
  const workspaceRule = { scope: 'workspace', workspace_id: 12 }
  assert.equal(permissionRuleMatchesScope(workspaceRule, 'workspace', null, null), false)
  assert.equal(permissionRuleMatchesScope(workspaceRule, 'workspace', 13, null), false)
  assert.equal(permissionRuleMatchesScope(workspaceRule, 'workspace', 12, null), true)
})

test('workspace edits cannot leak through the effective layer when no workspace is active', () => {
  assert.equal(permissionRuleMatchesScope({ scope: 'workspace', workspace_id: 12 }, 'effective', null, null), false)
  assert.equal(permissionRuleMatchesScope({ scope: 'global' }, 'effective', null, null), true)
  assert.equal(permissionRuleMatchesScope({ scope: 'workspace', workspace_id: 12 }, 'effective', 12, null), true)
})
