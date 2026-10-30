import { describe, expect, test } from 'bun:test'
import { hasToolAttachmentOutput, isDeferredAttachmentOutput, toolOutputAttachments } from '../src/lib/toolAttachments'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'
import { mediaAttachmentPresentation, operationPresentation } from '../src/pages/chat/transcriptPartPresentation'
import { attachmentPreviewUrl, attachmentDownloadUrl } from '../src/lib/attachmentLinks'

const attachment = {
  kind: 'image',
  mime: 'image/png',
  filename: 'generated.png',
  source: { source: 'local_path', path: '/state/generated_images/46/generated.png' },
}

describe('attachment output presentation', () => {
  test('deferred payload does not hide attachments or collapse their operation', () => {
    const output = { payload: null, attachments: [attachment], truncated: false }
    expect(toolOutputAttachments(output)).toEqual([attachment])
    expect(isDeferredAttachmentOutput(output)).toBe(true)
    expect(isDeferredAttachmentOutput({ ...output, payload: { text: 'complete' } })).toBe(false)
    expect(hasToolAttachmentOutput({ output }, {})).toBe(true)
    const display = projectLocalPart(
      { id: '1129', agenaKind: 'tool_call', agenaContent: { name: 'image.generate', output } },
      'assistant',
    )
    expect(display.defaultExpanded).toBe(true)
    const operation = operationPresentation(display)
    expect(operation.presentationBlocks).toHaveLength(1)
    const media = mediaAttachmentPresentation(operation.presentationBlocks[0]!)
    expect(media).toMatchObject({
      kind: 'image',
      mime: 'image/png',
      path: attachment.source.path,
      label: 'generated.png',
    })
  })

  test('modern media blocks and the same output attachment are not shown twice', () => {
    const display = projectLocalPart(
      {
        id: '1129',
        agenaKind: 'tool_call',
        agenaContent: { name: 'image.generate', output: { payload: null, attachments: [attachment] } },
        agenaPresentation: {
          blocks: [
            {
              type: 'media',
              artifact: { uri: `file://${attachment.source.path}`, mime: 'image/png', name: 'generated.png' },
            },
          ],
        },
      },
      'assistant',
    )
    expect(display.defaultExpanded).toBe(true)
    expect(operationPresentation(display).presentationBlocks).toHaveLength(1)
  })

  test('uploaded resource parts open by default; ordinary tools retain their prior default', () => {
    expect(
      projectLocalPart({ id: '1', agenaKind: 'file_ref', agenaContent: { attachments: [attachment] } }, 'user')
        .defaultExpanded,
    ).toBe(true)
    expect(
      projectLocalPart({ id: '2', agenaKind: 'tool_call', agenaContent: { name: 'shell.exec' } }, 'assistant')
        .defaultExpanded,
    ).toBe(false)
  })

  test('legacy audio, video and file blocks use the same attachment renderer', () => {
    for (const [kind, mime] of [
      ['audio', 'audio/wav'],
      ['video', 'video/webm'],
      ['file', 'application/pdf'],
    ] as const) {
      expect(
        mediaAttachmentPresentation({ type: kind, uri: '/api/v1/sessions/46/media?path=x', mime, filename: 'file' }),
      ).toMatchObject({ kind, mime, url: '/api/v1/sessions/46/media?path=x' })
    }
  })

  test.each([
    ['image', 'image/png', 'landscape.png'],
    ['image', 'image/svg+xml', 'chart.svg'],
    ['audio', 'audio/wav', 'audio.wav'],
    ['video', 'video/webm', 'video.webm'],
    ['file', 'application/pdf', 'report.pdf'],
  ])('%s output keeps one session-authorized attachment across Output reads and folds', (kind, mime, filename) => {
    const path = `/home/canxin/agena/ai-art/${filename}`
    const item = {
      kind,
      mime,
      filename,
      sha256: '823d5df7772c6aed3f983a9667efafed2c32d29405144485cc560d463a33c26b',
      size_bytes: 2849031,
      source: { source: 'local_path', path },
    }
    const output = { payload: null, attachments: [item], truncated: false }
    const display = projectLocalPart(
      {
        id: '1146',
        agenaKind: 'tool_call',
        partState: 'completed',
        agenaContent: { name: 'chatgpt.image_generate', output },
        agenaPresentation: {
          blocks: [
            { type: 'markdown', id: 'provider-meta', text: 'The generated result.' },
            { type: 'media', id: 'media', artifact: { uri: `file://${path}`, mime, name: filename } },
          ],
        },
      },
      'assistant',
    )
    expect(display.defaultExpanded).toBe(true)
    const complete = { ...output, payload: { text: 'The generated result.' } }
    for (const sections of [{}, { output: complete }, { output: null }, { output: complete, metadata: {} }]) {
      for (const expanded of [true, false, true]) {
        const blocks = operationPresentation(display, sections, expanded).presentationBlocks
        const attachments = blocks.map(mediaAttachmentPresentation).filter((item) => item !== null)
        expect(attachments).toHaveLength(expanded ? 1 : 0)
        if (!expanded) continue
        const media = attachments[0]!
        expect(media).toMatchObject({ path, mime, label: filename })
        const href = attachmentPreviewUrl({ ...media, sessionId: '48', workspaceRoot: '/workspace/repo' })
        const url = new URL(href, 'https://example.test')
        expect(url.pathname).toBe('/api/v1/sessions/48/media')
        expect(url.searchParams.get('path')).toBe(path)
        expect(url.searchParams.has('token')).toBe(false)
        expect(attachmentDownloadUrl({ ...media, sessionId: '48' })).toBe(`${href}&download=true`)
        expect(attachmentPreviewUrl({ ...media, sessionId: '49' })).not.toBe(href)
      }
    }
    expect(display.source.agenaContent.output).toEqual(output)
  })
})
