import { expect, test } from 'bun:test'
import {
  availableSavedPrompts,
  composedPromptText,
  groupSavedPrompts,
  nextSavedPromptId,
  promptExcerpt,
  savedPromptExcerpt,
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

test('category choices include empty categories and uncategorized prompts', () => {
  const groups = groupSavedPrompts({ ...globalConfig, categories: ['templates', 'research'] })
  expect(groups.map(({ category, prompts }) => [category, prompts.map(({ id }) => id)])).toEqual([
    ['research', ['shared']],
    ['templates', []],
    [null, ['global-only', 'legacy-inline']],
  ])
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
  const prompt = {
    id: 'valid',
    title: '中文研究',
    category: 'research',
    body: 'Check facts',
    when: { models: [], requires_tools: null },
  }
  expect(savedPromptIsValid(prompt)).toBe(true)
  expect(savedPromptMatches(prompt, '研究')).toBe(true)
  expect(savedPromptMatches(prompt, 'research')).toBe(true)
  expect(savedPromptMatches(prompt, 'unrelated')).toBe(false)
  expect(savedPromptIsValid({ ...prompt, body: '   ' })).toBe(false)
  expect(savedPromptIsValid({ ...prompt, body: '中'.repeat(6000) })).toBe(false)
})

test('prompt rows preview the first line of body text and bound long Unicode excerpts', () => {
  expect(promptExcerpt('\n## 写作约定\r\n\r\n  先说\t 结论。\n补充背景。')).toBe('先说 结论。…')
  expect(promptExcerpt('# 独立标题')).toBe('# 独立标题')
  expect(promptExcerpt('Short prompt.')).toBe('Short prompt.')
  expect(promptExcerpt('𠮷'.repeat(121))).toBe(`${'𠮷'.repeat(120)}…`)
  expect(promptExcerpt('   ')).toBe('')
})

test('compositions preview linked body text in reference order and keep the full detail', () => {
  const { library } = normalizePromptScopeConfig({
    library: [
      { id: 'ending', body: 'End with an example.' },
      { id: 'opening', body: 'Start with the result.' },
      { id: 'inner', parts: ['opening'] },
      { id: 'combined', parts: ['inner', 'ending'] },
    ],
  })
  const byId = new Map(library.map((prompt) => [prompt.id, prompt]))
  const composition = byId.get('combined')!
  expect(savedPromptExcerpt(composition, byId)).toBe('Start with the result.…')
  expect(composedPromptText(composition, byId)).toBe('Start with the result.\n\nEnd with an example.')
  byId.delete('opening')
  expect(savedPromptExcerpt(composition, byId)).toBe('')
})
