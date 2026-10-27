import { describe, expect, test } from 'bun:test'
import type { PluginCommandCatalogItem } from '../src/lib/pluginOperations'
import type { SkillSummary } from '../src/lib/skillsApi'
import {
  createSkillReferenceAttachment,
  isSkillCatalogEntry,
  managedSkillCatalog,
  skillCatalogKey,
  skillReferenceKeyFromAttachment,
} from '../src/pages/chat/skillReferences'
import { restoredComposerFiles } from '../src/pages/chat/queuedComposer'

function skill(overrides: Partial<SkillSummary> = {}): SkillSummary {
  return {
    id: 'workspace-opaque-id',
    name: 'code-review',
    description: 'Review changes in this project.',
    scope: 'workspace',
    path: '/project/.agents/skills/code-review/SKILL.md',
    editable: true,
    enabled: true,
    valid: true,
    shadowed: false,
    revision: 'package-revision-2',
    source: null,
    diagnostics: [],
    ...overrides,
  }
}

const legacy: PluginCommandCatalogItem = {
  plugin_id: 'legacy.commands',
  id: 'review',
  title: 'Review code',
  group: 'Skills',
  category: ' skill ',
  aliases: ['check'],
  docs: { summary: 'Legacy review instructions' },
  accepts_empty_input: true,
  default_input: {},
  input: { version: 1, root: { id: 'input', path: '', title: 'Input', kind: 'object', fields: [] } },
  target: { kind: 'method', handler: 'review' },
}

describe('skill references', () => {
  test('offers only active entries from the current catalog, including read-only compatible skills', () => {
    const entries = managedSkillCatalog([
      skill({ id: 'disabled', enabled: false }),
      skill({ id: 'invalid', valid: false }),
      skill({ id: 'overridden', scope: 'global', shadowed: true }),
      skill({ id: 'compatible', editable: false, name: 'compatible-review' }),
      skill(),
    ])
    expect(entries.map((entry) => entry.skill_id)).toEqual(['workspace-opaque-id', 'compatible'])
    expect(entries.every((entry) => !('target' in entry) && !('input' in entry))).toBe(true)
  })

  test('sends an exact package ID and revision with a human-readable name, without inline instructions', async () => {
    const [entry] = managedSkillCatalog([skill()])
    const reference = await createSkillReferenceAttachment(entry!)
    expect(reference.filename).toBe('Skill: code-review')
    expect(reference.delivery).toBe('reference')
    expect(reference.composerNode).toEqual({
      type: 'activity',
      activity: {
        id: reference.id,
        payload: {
          activity_type: 'command_reference',
          name: 'code-review',
          skill_id: 'workspace-opaque-id',
          description: 'Review changes in this project.',
          content_hash: 'package-revision-2',
          source: 'agena.skills',
          aliases: [],
        },
        provenance: {
          source: 'agena.skills',
          content_hash: 'package-revision-2',
          plugin_id: 'agena.skills',
        },
      },
    })
    expect(skillReferenceKeyFromAttachment(reference)).toBe(skillCatalogKey(entry!))
  })

  test('keeps same-name packages distinct and preserves identity when restoring queued drafts', async () => {
    const entries = managedSkillCatalog([
      skill(),
      skill({ id: 'global-opaque-id', scope: 'global', path: '/global/code-review/SKILL.md' }),
    ])
    const references = await Promise.all(entries.map(createSkillReferenceAttachment))
    expect(new Set(references.map(skillReferenceKeyFromAttachment)).size).toBe(2)
    expect(references.map((file) => file.filename)).toEqual(['Skill: code-review', 'Skill: code-review'])
    const restored = restoredComposerFiles(references.map((file) => file.composerNode!))
    expect(restored.map(skillReferenceKeyFromAttachment)).toEqual(references.map(skillReferenceKeyFromAttachment))
    expect(restored.map((file) => file.composerNode)).toEqual(references.map((file) => file.composerNode))
  })

  test('retains legacy command-reference metadata and definition hashes without adding a package ID', async () => {
    expect(isSkillCatalogEntry(legacy)).toBe(true)
    const reference = await createSkillReferenceAttachment(legacy)
    const restored = restoredComposerFiles([reference.composerNode!])[0]!
    expect(skillReferenceKeyFromAttachment(restored)).toBe('legacy.commands/review')
    expect(reference.composerNode).toEqual({
      type: 'activity',
      activity: {
        id: reference.id,
        payload: {
          activity_type: 'command_reference',
          name: 'review',
          description: 'Legacy review instructions',
          content_hash: expect.stringMatching(/^[a-f0-9]{64}$/),
          source: 'legacy.commands',
          aliases: ['check'],
        },
        provenance: {
          source: 'legacy.commands',
          content_hash: expect.stringMatching(/^[a-f0-9]{64}$/),
          plugin_id: 'legacy.commands',
        },
      },
    })
  })
})
