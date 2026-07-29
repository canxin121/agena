import type { PluginUiToolInvokeResponse, SkillReferenceInput } from '../lib/agenaApi'

export const SKILL_PICKER_PAGE_SIZE = 12
export const MAX_COMPOSER_SKILLS = 8

export type SkillCatalogItem = {
  name: string
  summary: string
  aliases: string[]
  source: string
  contentHash: string
}

export type SkillCatalogPage = {
  items: SkillCatalogItem[]
  total: number
  offset: number
  returned: number
}

export type ComposerSkillDraft = {
  id: string
  name: string
  description: string
  source: string
  contentHash: string
  item: SkillReferenceInput
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function readString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function readStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.map(readString).filter((entry): entry is string => Boolean(entry)) : []
}

function readCount(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : 0
}

export function parseSkillCatalogPage(response: PluginUiToolInvokeResponse): SkillCatalogPage {
  const payload = asRecord(response.payload)
  if (!payload) throw new Error('Skill catalog returned no structured payload.')
  const rawItems = Array.isArray(payload.tools) ? payload.tools : []
  const items = rawItems.flatMap((value) => {
    const item = asRecord(value)
    if (!item || readString(item.kind) !== 'skill') return []
    const name = readString(item.name)
    const contentHash = readString(item.content_hash)
    const source = readString(item.source)
    if (!name || !contentHash || !source) return []
    return [
      {
        name,
        summary: readString(item.summary),
        aliases: readStrings(item.aliases),
        source,
        contentHash,
      },
    ]
  })
  return {
    items,
    total: readCount(payload.total),
    offset: readCount(payload.offset),
    returned: readCount(payload.returned),
  }
}

export function createComposerSkillDraft(response: PluginUiToolInvokeResponse): ComposerSkillDraft {
  const payload = asRecord(response.payload)
  if (!payload) throw new Error('Skill detail returned no structured payload.')
  if (readString(payload.kind) !== 'skill') throw new Error('The selected runtime entry is not a Skill.')

  const name = readString(payload.name)
  const description = readString(payload.summary)
  const instructions = readString(payload.body)
  const contentHash = readString(payload.content_hash)
  const source = readString(payload.source)
  if (!name || !instructions || !contentHash || !source) {
    throw new Error('Skill detail is missing its name, instructions, content hash, or source.')
  }

  const item: SkillReferenceInput = {
    name,
    description,
    instructions,
    content_hash: contentHash,
    source,
    aliases: readStrings(payload.aliases),
  }
  return {
    id: `${name}:${contentHash}`,
    name,
    description,
    source,
    contentHash,
    item,
  }
}
