import { expect, test } from 'bun:test'
import { reactive } from 'vue'
import {
  agentProfileCatalog,
  agentProfileInstructions,
  agentProfileLineage,
  agentProfileReferences,
  isValidAgentProfileName,
  mergedAgentProfiles,
  withAgentProfileInstructions,
} from '../src/lib/agentProfiles'
import { normalizePromptLayer } from '../src/lib/promptLayers'
import { buildCreateSessionRequest } from '../src/stores/chat/api'
import { buildSettingsScopes } from '../src/components/settings/settingsScopeCatalog'
import { buildSettingsNavigationCategories } from '../src/components/settings/settingsInformationArchitecture'

test('workspace profiles replace whole named entries without retaining global grants', () => {
  const global = { developer: { permission: { path: { workspace: 'readwrite' } }, description: 'Global' } }
  const workspace = { developer: { prompts: { order: ['review'] } } }
  expect(mergedAgentProfiles(global, workspace)).toEqual(workspace)
  expect(global.developer.permission.path.workspace).toBe('readwrite')
})

test('settings values are copied safely from reactive API responses', () => {
  const value = reactive({ review: { title: 'Reviewer', permission: { path: { workspace: 'readonly' } } } })
  const copied = agentProfileCatalog(value)
  copied.review!.title = 'Changed'
  expect(value.review.title).toBe('Reviewer')
})

test('inheritance resolves parent first and detects missing and circular parents', () => {
  const catalog = { base: {}, review: { extends: 'base' }, strict: { extends: 'review' } }
  expect(agentProfileLineage(catalog, 'strict')).toEqual(['base', 'review', 'strict'])
  expect(agentProfileReferences(catalog, 'base')).toEqual(['review'])
  expect(() => agentProfileLineage({ review: { extends: 'missing' } }, 'review')).toThrow('Unknown profile: missing')
  expect(() => agentProfileLineage({ a: { extends: 'b' }, b: { extends: 'a' } }, 'a')).toThrow('cycle')
})

test('the body editor preserves saved references, mode, disabled entries and conditions', () => {
  const profile = {
    prompts: normalizePromptLayer({
      mode: 'replace',
      order: ['saved-style', 'custom'],
      disabled: ['old'],
      inline: [{ id: 'custom', body: 'Original', title: 'Custom instructions', when: { models: ['local/*'] } }],
    }),
  }
  const changed = withAgentProfileInstructions('review', profile, 'Review carefully')
  expect(changed.prompts?.order).toEqual(['saved-style', 'custom'])
  expect(changed.prompts?.mode).toBe('replace')
  expect(changed.prompts?.disabled).toEqual(['old'])
  expect(changed.prompts?.inline?.[0]?.when.models).toEqual(['local/*'])
  expect(agentProfileInstructions('review', changed)).toBe('Review carefully')
  expect(profile.prompts.inline[0]!.body).toBe('Original')
  const cleared = withAgentProfileInstructions('review', changed, '')
  expect(cleared.prompts?.inline).toEqual([])
  expect(cleared.prompts?.order).toEqual(['saved-style'])
})

test('long profile names produce valid, distinct instruction ids', () => {
  const prefix = 'a'.repeat(63)
  const first = withAgentProfileInstructions(prefix + 'b', {}, 'First')
  const second = withAgentProfileInstructions(prefix + 'c', {}, 'Second')
  const firstId = first.prompts!.order![0]!
  const secondId = second.prompts!.order![0]!
  expect(firstId).not.toBe(secondId)
  expect(isValidAgentProfileName(firstId)).toBe(true)
  expect(isValidAgentProfileName(secondId)).toBe(true)
  expect(isValidAgentProfileName('.review')).toBe(false)
  expect(isValidAgentProfileName('Review')).toBe(false)
})

test('session create preserves default, explicit selection, and explicit no-profile states', () => {
  const input = { workspaceId: 7, title: 'Review' }
  expect(buildCreateSessionRequest(input)).not.toHaveProperty('profile')
  expect(buildCreateSessionRequest({ ...input, profile: 'review' }).profile).toBe('review')
  expect(buildCreateSessionRequest({ ...input, profile: '' }).profile).toBe('')
})

test('agent profiles have a distinct AI settings entry and a current-session scope', () => {
  const category = buildSettingsNavigationCategories().find((item) => item.id === 'ai-session')!
  expect(category.children?.find((item) => item.id === 'agent-profiles')?.view).toBe('agent-profiles')
  expect(buildSettingsScopes('runtime-session', 'agent-profiles').map((scope) => scope.id)).toEqual([
    'global',
    'workspace',
    'session',
  ])
})
