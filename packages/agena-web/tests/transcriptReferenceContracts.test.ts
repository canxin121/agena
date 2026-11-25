import { describe, expect, test } from 'bun:test'
import { projectLocalPart, projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'
import { attachmentPresentations, skillPresentations } from '../src/pages/chat/transcriptPartPresentation'

describe('current transcript reference contracts', () => {
  test('composer activities project to the same attachment and skill envelopes as persisted parts', () => {
    const skill = {
      activity_type: 'skill_reference',
      name: 'review',
      description: 'Review the patch',
      source: 'agena.skills',
      content_hash: 'skill-revision',
    }
    const resource = {
      activity_type: 'resource',
      kind: 'image',
      name: 'screenshot.png',
      media_type: 'image/png',
      reference: { reference_type: 'workspace_path', path: 'screenshots/current.png' },
      size_bytes: 42,
    }
    const [message] = projectTranscriptBlocks([
      {
        info: { id: '1', role: 'user' },
        parts: [
          {
            id: '1',
            agenaKind: 'text',
            agenaContent: {
              text: 'Review this',
              document: [
                { type: 'activity', activity: { id: 'attachment', payload: resource } },
                { type: 'activity', activity: { id: 'skill', payload: skill } },
              ],
            },
          },
        ],
      },
    ])
    const attachment = message!.displayParts.find((part) => part.kind === 'resource')!
    const reference = message!.displayParts.find((part) => part.kind === 'skill')!
    expect(Object.keys(attachment.source.agenaContent!)).toEqual(['attachments'])
    expect(attachmentPresentations(attachment)).toEqual([
      expect.objectContaining({
        label: 'screenshot.png',
        mime: 'image/png',
        path: 'screenshots/current.png',
        sizeBytes: 42,
      }),
    ])
    expect(reference.source.agenaContent).toEqual({ skills: [skill] })
    expect(skillPresentations(reference)).toEqual([
      { name: 'review', description: 'Review the patch', source: 'agena.skills', contentHash: 'skill-revision' },
    ])
    expect(reference.summary).toContain('review')
  })

  test('artifact and provider-file resources retain their current reference identity and access boundary', () => {
    const hash = 'a'.repeat(64)
    const fixtures: Array<{
      reference: Record<string, string>
      source: Record<string, string>
      key: string
      url: string
    }> = [
      {
        reference: { reference_type: 'artifact', uri: 'agena:media:v1:fixture', sha256: hash },
        source: { source: 'url', url: 'agena:media:v1:fixture' },
        key: hash,
        url: 'agena:media:v1:fixture',
      },
      {
        reference: { reference_type: 'provider_file', provider_id: 'fixture-provider', file_id: 'provider-upload-id' },
        source: { source: 'file_id', file_id: 'provider-upload-id' },
        key: 'screenshot.png:0',
        url: '',
      },
    ]
    for (const { reference, source, key, url } of fixtures) {
      const resource = {
        activity_type: 'resource',
        kind: 'image',
        name: 'screenshot.png',
        media_type: 'image/png',
        reference,
        size_bytes: 42,
      }
      const [message] = projectTranscriptBlocks([
        {
          info: { id: '1', role: 'user' },
          parts: [
            {
              id: '1',
              agenaKind: 'text',
              agenaContent: {
                document: [{ type: 'activity', activity: { id: 'attachment', payload: resource } }],
              },
            },
          ],
        },
      ])
      const attachment = message!.displayParts.find((part) => part.kind === 'resource')!
      expect(attachment.source.agenaContent).toEqual({
        attachments: [expect.objectContaining({ reference, source, sha256: reference.sha256 ?? null })],
      })
      expect(attachmentPresentations(attachment)).toEqual([
        expect.objectContaining({
          key,
          label: 'screenshot.png',
          kind: 'image',
          mime: 'image/png',
          url,
          path: '',
          sizeBytes: 42,
          unavailable: false,
        }),
      ])
    }
  })

  test('incomplete top-level attachment and skill shapes are not upgraded', () => {
    const attachment = projectLocalPart(
      {
        id: 'old-file',
        agenaKind: 'file_ref',
        filename: 'old.png',
        url: '/old.png',
        agenaContent: { mime: 'image/png', path: '/old.png' },
      },
      'user',
    )
    expect(attachmentPresentations(attachment)).toEqual([])
    expect(attachment.summary).not.toContain('old.png')
    for (const content of [{ skill: 'old-skill' }, { skills: [{ source: 'incomplete' }] }]) {
      const skill = projectLocalPart({ id: 'old-skill', agenaKind: 'skill_ref', agenaContent: content }, 'user')
      expect(skillPresentations(skill)).toEqual([])
      expect(skill.summary).not.toContain('old-skill')
    }
  })
})
