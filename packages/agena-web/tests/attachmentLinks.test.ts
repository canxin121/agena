import { describe, expect, test } from 'bun:test'
import { attachmentDownloadUrl, attachmentPreviewUrl } from '../src/lib/attachmentLinks'

describe('session-scoped attachment links', () => {
  test('immutable media stays opaque and is scoped to the displayed session', () => {
    const uri = 'agena:media:v1:7b22626c6f62223a2278227d'
    const url = attachmentPreviewUrl({ url: uri, sessionId: '43' })
    expect(url).toStartWith('/api/v1/sessions/43/media?')
    expect(new URL(url, 'https://localhost').searchParams.get('path')).toBe(uri)
    expect(attachmentPreviewUrl({ url: uri })).toBe('')
    expect(attachmentDownloadUrl({ url: uri, sessionId: 43 })).toEndWith('&download=true')
  })
  test('generated images and uploaded files do not use the workbench raw route', () => {
    for (const path of [
      '/data/workspaces/x/generated_images/43/image.png',
      '/data/workspaces/x/uploads/图 #1.png',
      'docs/report.pdf',
    ]) {
      const url = attachmentPreviewUrl({ path, sessionId: 43, workspaceRoot: '/project' })
      expect(url).toStartWith('/api/v1/sessions/43/media?')
      expect(new URL(url, 'https://localhost').searchParams.get('path')).toBe(path)
    }
  })
  test('file URIs, no-session workspace references and browser URLs are handled', () => {
    expect(attachmentPreviewUrl({ url: 'file:///tmp/image.png', sessionId: 1 })).toContain('path=%2Ftmp%2Fimage.png')
    expect(attachmentPreviewUrl({ path: 'a.txt', workspaceRoot: '/repo' })).toStartWith('/api/v1/workbench/fs/raw?')
    for (const url of ['https://example.org/image.png', 'blob:https://localhost/123', 'data:image/png;base64,AAA=']) {
      expect(attachmentPreviewUrl({ url })).toBe(url)
    }
  })
  test('missing content and executable schemes cannot be navigated', () => {
    expect(attachmentPreviewUrl({ path: '/private', sessionId: '../other' })).toBe('')
    expect(attachmentPreviewUrl({ url: 'javascript:alert(1)' })).toBe('')
    expect(attachmentPreviewUrl({ url: 'file-id:secret' })).toBe('')
    expect(attachmentPreviewUrl({ path: '/x', sessionId: 43, unavailable: true })).toBe('')
  })
})

test('file dock never resolves managed attachments or old sessions in the active workspace', async () => {
  const { workspaceAttachmentPath } = await import('../src/lib/attachmentLinks')
  expect(workspaceAttachmentPath('src/main.rs', '/repo', '/repo')).toBe('src/main.rs')
  expect(workspaceAttachmentPath('/etc/example', '/', '/')).toBe('etc/example')
  expect(workspaceAttachmentPath('/repo//a.txt', '/repo', '/repo')).toBe('a.txt')
  expect(workspaceAttachmentPath('/repo/docs/a.pdf', '/repo', '/repo')).toBe('docs/a.pdf')
  expect(workspaceAttachmentPath('/repo-other/a.pdf', '/repo', '/repo')).toBeNull()
  expect(workspaceAttachmentPath('/state/uploads/image.png', '/repo', '/repo')).toBeNull()
  expect(workspaceAttachmentPath('src/main.rs', '/old-repo', '/new-repo')).toBeNull()
  expect(workspaceAttachmentPath('../secret', '/repo', '/repo')).toBeNull()
  expect(workspaceAttachmentPath('C:\\repo\\a.txt', 'C:\\repo', 'C:\\repo')).toBe('a.txt')
})
