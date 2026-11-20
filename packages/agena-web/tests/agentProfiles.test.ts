import { expect, test } from 'bun:test'
import { reactive } from 'vue'
import {
  agentProfileCatalog,
  agentProfileInstructions,
  agentProfileLineage,
  agentProfileMetadata,
  selectableAgentProfiles,
  agentProfileReferences,
  isValidAgentProfileName,
  mergedAgentProfiles,
  withBuiltinAgentProfiles,
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

test('role modes and model visibility inherit independently with all as the legacy default', () => {
  const catalog = {
    main: { mode: 'primary' as const },
    explore: { mode: 'subagent' as const, hidden: true },
    custom: { extends: 'explore' },
    public: { extends: 'custom', hidden: false },
    reviewer: { mode: 'all' as const },
    legacy: {},
  }
  expect(agentProfileMetadata(catalog, 'custom')).toEqual({ mode: 'subagent', hidden: true })
  expect(agentProfileMetadata(catalog, 'public')).toEqual({ mode: 'subagent', hidden: false })
  expect(agentProfileMetadata(catalog, 'legacy')).toEqual({ mode: 'all', hidden: false })
  expect(Object.keys(selectableAgentProfiles(catalog, false))).toEqual(['main', 'reviewer', 'legacy'])
  expect(Object.keys(selectableAgentProfiles(catalog, true))).toEqual([
    'explore',
    'custom',
    'public',
    'reviewer',
    'legacy',
  ])
  expect(() => agentProfileMetadata({ bad: { mode: 'invalid' as any } }, 'bad')).toThrow('Invalid profile mode')
  expect(() => agentProfileMetadata({ bad: { hidden: 'yes' as any } }, 'bad')).toThrow('boolean')
})

test('partial built-in overrides retain role behavior while file scopes replace raw named entries', () => {
  const builtins = {
    explore: {
      title: 'Explore',
      description: 'Search the code',
      mode: 'subagent' as const,
      permission: { path: { workspace: 'readonly', external: 'readonly' } },
      prompts: { order: ['explore'], inline: [{ id: 'explore', body: 'Inspect and report' }] },
    },
  }
  const merged = withBuiltinAgentProfiles(builtins, { explore: { model: { model: 'fast' } } })
  expect(merged.explore).toMatchObject({ mode: 'subagent', description: 'Search the code', model: { model: 'fast' } })
  expect(merged.explore!.permission).toEqual(builtins.explore.permission)
  expect(merged.explore!.prompts).toEqual(builtins.explore.prompts)
  const raw = mergedAgentProfiles(
    { explore: { description: 'Global override', hidden: true } },
    { explore: { model: { model: 'project' } } },
  )
  const scoped = withBuiltinAgentProfiles(builtins, raw)
  expect(scoped.explore!.description).toBe('Search the code')
  expect(scoped.explore!.hidden).toBeUndefined()
  expect(scoped.explore!.model).toEqual({ model: 'project' })
})

test('built-in prompt overlays honor disabled fragments, order changes and explicit replacement', () => {
  const builtins = {
    main: {
      prompts: {
        order: ['one', 'two'],
        inline: [
          { id: 'one', body: 'One' },
          { id: 'two', body: 'Two' },
        ],
      },
      permission: {
        path: { workspace: 'readonly', rules: { '/old': 'deny' } },
        network: { default: 'deny' },
        tools: { names: { 'agena.shell.exec': 'ask' } },
      },
    },
  }
  const merged = withBuiltinAgentProfiles(builtins, {
    main: {
      prompts: {
        disabled: ['one'],
        order: ['custom', 'two'],
        inline: [
          { id: 'custom', body: 'Custom' },
          { id: 'two', body: 'New two' },
        ],
      },
      permission: { path: { rules: { '/new': 'readonly' } }, network: { default: 'allow' } },
    },
  })
  expect(merged.main!.prompts!.order).toEqual(['custom', 'two'])
  expect(merged.main!.prompts!.inline!.map((fragment) => fragment.body)).toEqual(['Custom', 'New two'])
  expect(merged.main!.permission).toEqual({
    path: { workspace: 'readonly', rules: { '/old': 'deny', '/new': 'readonly' } },
    network: { version: 2, default: 'allow', inbound: 'deny', hostLoopback: 'deny', allow: [], deny: [] },
    tools: { names: { 'agena.shell.exec': 'ask' } },
  })
  const replaced = withBuiltinAgentProfiles(builtins, { main: { prompts: { mode: 'replace' } } })
  expect(replaced.main!.prompts!.inline).toEqual([])
  expect(replaced.main!.prompts!.order).toEqual([])
})
