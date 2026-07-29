import { describe, expect, test } from 'bun:test'

import type { PluginUiToolInvokeResponse } from '../lib/agenaApi'
import { createComposerSkillDraft, parseSkillCatalogPage } from './chatSkillModel'

function response(payload: unknown): PluginUiToolInvokeResponse {
  return {
    plugin_id: 'agena.skills',
    tool: 'agena.skills.test',
    title: 'Skills',
    output_text: '',
    payload,
  }
}

describe('chatSkillModel', () => {
  test('parses the paginated real Skill catalog and excludes command entries', () => {
    const page = parseSkillCatalogPage(
      response({
        tools: [
          {
            name: 'review',
            kind: 'skill',
            summary: 'Review changes',
            aliases: ['code-review'],
            allowed_tools: ['agena.repo.diff'],
            source: 'bundled',
            content_hash: 'abc123',
          },
          {
            name: 'commit',
            kind: 'command',
            summary: 'Commit changes',
            source: 'filesystem',
            content_hash: 'ignored',
          },
        ],
        total: 14,
        offset: 12,
        returned: 2,
      }),
    )

    expect(page.total).toBe(14)
    expect(page.offset).toBe(12)
    expect(page.items).toEqual([
      {
        name: 'review',
        summary: 'Review changes',
        aliases: ['code-review'],
        allowedTools: ['agena.repo.diff'],
        source: 'bundled',
        contentHash: 'abc123',
      },
    ])
  })

  test('creates an immutable composer snapshot from Skill detail', () => {
    const draft = createComposerSkillDraft(
      response({
        name: 'review',
        kind: 'skill',
        summary: 'Review changes',
        body: 'Inspect the diff and report findings.',
        aliases: ['code-review'],
        allowed_tools: ['agena.repo.diff'],
        source: 'bundled',
        content_hash: 'abc123',
      }),
    )

    expect(draft.id).toBe('review:abc123')
    expect(draft.item).toEqual({
      name: 'review',
      description: 'Review changes',
      instructions: 'Inspect the diff and report findings.',
      content_hash: 'abc123',
      source: 'bundled',
      aliases: ['code-review'],
      allowed_tools: ['agena.repo.diff'],
    })
  })

  test('rejects incomplete Skill detail instead of attaching an ambiguous reference', () => {
    let message = ''
    try {
      createComposerSkillDraft(
        response({
          name: 'review',
          kind: 'skill',
          body: '',
          source: 'bundled',
          content_hash: 'abc123',
        }),
      )
    } catch (error) {
      message = error instanceof Error ? error.message : String(error)
    }
    expect(message).toContain('missing its name, instructions, content hash, or source')
  })
})
