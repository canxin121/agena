import { expect, test } from 'bun:test'
import {
  availableSavedPrompts,
  nextSavedPromptId,
  savedPromptIsValid,
  savedPromptMatches,
  scopeUsesPrompt,
} from '../src/lib/promptLibrary'
import { normalizePromptScopeConfig } from '../src/lib/promptLayers'

const globalConfig = normalizePromptScopeConfig({
  library: [
    { id: 'shared', title: 'Global shared', category: 'research', body: 'Global content' },
    { id: 'global-only', title: 'Global only', body: 'Always global' },
  ],
  inline: [{ id: 'legacy-inline', title: 'Legacy', body: 'Old content' }],
})
const workspaceConfig = normalizePromptScopeConfig({
  library: [
    { id: 'shared', title: 'Workspace shared', body: 'Workspace content' },
    { id: 'workspace-only', title: 'Workspace only', body: 'Workspace instruction' },
  ],
  order: ['shared'],
})

test('saved library entries are separate from active layer order', () => {
  expect(globalConfig.order).toEqual([])
  expect(globalConfig.library).toHaveLength(2)
  expect(scopeUsesPrompt(globalConfig, 'shared')).toBe(false)
  expect(scopeUsesPrompt(workspaceConfig, 'shared')).toBe(true)
})

test('attachment chooser shows names across scopes with workspace precedence', () => {
  const global = availableSavedPrompts(globalConfig, workspaceConfig, 'global')
  expect(global.find((item) => item.id === 'shared')?.fragment.title).toBe('Global shared')
  expect(global.some((item) => item.id === 'workspace-only')).toBe(false)

  const session = availableSavedPrompts(globalConfig, workspaceConfig, 'session')
  expect(session.find((item) => item.id === 'shared')?.fragment.title).toBe('Workspace shared')
  expect(session.find((item) => item.id === 'legacy-inline')?.legacy).toBe(true)
  expect(session.filter((item) => item.id === 'shared')).toHaveLength(1)
})

test('users do not have to invent or type internal prompt ids', () => {
  expect(nextSavedPromptId('global', globalConfig, workspaceConfig)).toBe('saved-global-1')
  const used = normalizePromptScopeConfig({ library: [{ id: 'saved-global-1', title: 'First', body: 'A' }] })
  expect(nextSavedPromptId('global', used, workspaceConfig)).toBe('saved-global-2')
})

test('search and validation handle custom categories, empty bodies and UTF-8 size', () => {
  const prompt = { id: 'valid', title: '中文研究', category: 'research', body: 'Check facts', when: { models: [], requires_tools: null } }
  expect(savedPromptIsValid(prompt)).toBe(true)
  expect(savedPromptMatches(prompt, '研究')).toBe(true)
  expect(savedPromptMatches(prompt, 'research')).toBe(true)
  expect(savedPromptMatches(prompt, 'unrelated')).toBe(false)
  expect(savedPromptIsValid({ ...prompt, body: '   ' })).toBe(false)
  expect(savedPromptIsValid({ ...prompt, body: '中'.repeat(6000) })).toBe(false)
})
