import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import { normalizePromptFragment, type BuiltinPrompt } from '@/lib/promptLayers'

export function readBuiltinPrompts(signal?: AbortSignal): Promise<BuiltinPrompt[]> {
  return apiJson('/api/v1/prompts/builtins?include_reusable=true', { signal })
}

export function reusableBuiltinPrompts(catalog: readonly BuiltinPrompt[]) {
  return catalog
    .filter((prompt) => prompt.reusable)
    .map((prompt) =>
      normalizePromptFragment({
        id: prompt.id,
        title: builtinPromptTitle(prompt),
        category: st('Built-in prompts'),
        body: prompt.body,
      }),
    )
}

export function builtinPromptTitle(prompt: BuiltinPrompt) {
  switch (prompt.id) {
    case 'agena.system.identity':
      return prompt.aliases?.length ? st('Assistant identity and shared rules') : st('Assistant identity')
    case 'agena.system.evidence':
      return st('Runtime facts and evidence')
    case 'agena.system.conversation':
      return st('User-facing session')
    case 'agena.system.delegation':
      return st('Delegated session')
    case 'agena.system.tools':
      return st('Tool protocol')
    case 'agena.system.authorization':
      return st('Authorization and protecting work')
    case 'agena.system.context':
      return st('Context and project guidance')
    case 'agena.system.communication':
      return st('Communication and delivery')
    case 'agena.system.markdown':
      return st('Markdown presentation')
    case 'agena.system.rich-output':
      return st('Rich output and inline UI')
    case 'agena.profile.investigation':
      return st('Investigation and evidence')
    case 'agena.profile.implementation':
      return st('Implementation and verification')
    case 'agena.profile.read-only':
      return st('Read-only work')
    case 'agena.profile.planning':
      return st('Planning and acceptance criteria')
    case 'agena.profile.verification':
      return st('Verification and evidence')
    default:
      return prompt.title
  }
}

export function builtinPromptApplicability(when: BuiltinPrompt['when']) {
  switch (when) {
    case 'always':
      return st('Every session')
    case 'user_facing':
      return st('User-facing sessions')
    case 'subagent':
      return st('Delegated sessions')
    case 'tools':
      return st('When tools are available')
    case 'user_facing_tools':
      return st('User-facing sessions with tools')
  }
}
