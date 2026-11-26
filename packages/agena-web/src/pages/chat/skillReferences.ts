import type { SkillSummary } from '@/lib/skillsApi'
import type { ExternalAgent, ExternalAgentCommand } from '@/types/chat'
import type { AttachedFile } from './useChatAttachments'

export type SkillCatalogEntry = {
  plugin_id: 'agena.skills' | `acp:${ExternalAgent}`
  id: string
  skill_id: string
  title: string
  docs: { summary: string }
  revision: string
  external_agent?: ExternalAgent
}

export function nativeSkillCatalog(agent: ExternalAgent, commands: readonly ExternalAgentCommand[]): SkillCatalogEntry[] {
  return commands.filter((command) => command.skill && command.skill_inline).map((command) => ({
    plugin_id: `acp:${agent}`,
    id: command.name,
    skill_id: command.name,
    title: command.name,
    docs: { summary: command.description },
    revision: '',
    external_agent: agent,
  }))
}

export function managedSkillCatalog(skills: SkillSummary[]): SkillCatalogEntry[] {
  return skills
    .filter((skill) => skill.enabled && skill.valid && !skill.shadowed)
    .map(
      (skill): SkillCatalogEntry => ({
        plugin_id: 'agena.skills',
        id: skill.name,
        skill_id: skill.id,
        title: skill.name,
        docs: { summary: skill.description },
        revision: skill.revision,
      }),
    )
    .sort((a, b) => a.title.localeCompare(b.title) || a.skill_id.localeCompare(b.skill_id))
}

export function isSkillReferenceType(value: unknown): boolean {
  return value === 'skill_reference' || value === 'command_reference'
}

export function skillCatalogKey(skill: SkillCatalogEntry): string {
  return `${skill.plugin_id}/${skill.skill_id}`
}

export function skillReferenceKeyFromAttachment(file: AttachedFile): string | null {
  const node = file.composerNode
  if (!node || typeof node !== 'object' || Array.isArray(node)) return null
  const activity = node.activity
  if (!activity || typeof activity !== 'object' || Array.isArray(activity)) return null
  const payload = activity.payload
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return null
  if (!isSkillReferenceType(payload.activity_type)) return null
  const skillId = typeof payload.skill_id === 'string' ? payload.skill_id.trim() : ''
  const name = skillId || (typeof payload.name === 'string' ? payload.name.trim() : '')
  const source = typeof payload.source === 'string' ? payload.source.trim() : ''
  return name && source ? `${source}/${name}` : null
}

export function createSkillReferenceAttachment(skill: SkillCatalogEntry): AttachedFile {
  const pluginId = skill.plugin_id
  const contentHash = skill.revision
  const id = crypto.randomUUID()

  return {
    id,
    filename: `Skill: ${skill.title || skill.id}`,
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
          activity_type: 'skill_reference',
          name: skill.id,
          ...(skill.external_agent ? { external_agent: skill.external_agent } : { skill_id: skill.skill_id }),
          description: skill.docs.summary.trim(),
          content_hash: contentHash,
          source: pluginId,
        },
        provenance: {
          source: pluginId,
          ...(!skill.external_agent && { content_hash: contentHash, plugin_id: pluginId }),
        },
      },
    },
  }
}
