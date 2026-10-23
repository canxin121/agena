import type { PluginCommandCatalogItem } from '@/lib/pluginOperations'
import type { AttachedFile } from './useChatAttachments'

export type SkillCatalogEntry = PluginCommandCatalogItem

export function isSkillCatalogEntry(command: PluginCommandCatalogItem): boolean {
  return command.category?.trim().toLowerCase() === 'skill'
}

export function skillCatalogKey(command: PluginCommandCatalogItem): string {
  return `${command.plugin_id}/${command.id}`
}

export function skillReferenceKeyFromAttachment(file: AttachedFile): string | null {
  const node = file.composerNode
  if (!node || typeof node !== 'object' || Array.isArray(node)) return null
  const activity = node.activity
  if (!activity || typeof activity !== 'object' || Array.isArray(activity)) return null
  const payload = activity.payload
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return null
  if (payload.activity_type !== 'command_reference') return null
  const name = typeof payload.name === 'string' ? payload.name.trim() : ''
  const source = typeof payload.source === 'string' ? payload.source.trim() : ''
  return name && source ? `${source}/${name}` : null
}

export async function createSkillReferenceAttachment(command: SkillCatalogEntry): Promise<AttachedFile> {
  const pluginId = command.plugin_id
  const definition = Object.fromEntries(
    Object.entries(command).filter(
      ([key]) => !['plugin_id', 'accepts_empty_input', 'default_input'].includes(key),
    ),
  )
  const serialized = JSON.stringify(definition)
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(serialized))
  const contentHash = [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('')
  const id = crypto.randomUUID()
  const description = String(command.docs?.summary || '').trim()
  const aliases = Array.isArray(command.aliases) ? command.aliases : []

  return {
    id,
    filename: `Skill: ${command.title || command.id}`,
    size: 0,
    mime: 'application/vnd.agena.skill-reference+json',
    composerKind: 'skill_ref',
    delivery: 'reference',
    state: 'ready',
    composerNode: {
      type: 'activity',
      activity: {
        id,
        payload: {
          activity_type: 'command_reference',
          name: command.id,
          description,
          content_hash: contentHash,
          source: pluginId,
          aliases,
        },
        provenance: {
          source: pluginId,
          content_hash: contentHash,
          plugin_id: pluginId,
        },
      },
    },
  }
}
