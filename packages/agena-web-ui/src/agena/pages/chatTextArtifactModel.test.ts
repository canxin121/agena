import { describe, expect, test } from 'bun:test'

import {
  composerTextArtifactPreview,
  createComposerTextArtifactDraft,
  textArtifactPlaceholder,
  TEXT_ARTIFACT_PASTE_THRESHOLD,
} from './chatTextArtifactModel'

describe('chatTextArtifactModel', () => {
  test('keeps pasted content in the artifact and renders a body placeholder', () => {
    const draft = createComposerTextArtifactDraft('x'.repeat(1200))
    expect(draft.text.length).toBe(1200)
    expect(Boolean(draft.id)).toBe(true)
    expect(textArtifactPlaceholder(1)).toBe('[已粘贴文本 #1]')
    expect(TEXT_ARTIFACT_PASTE_THRESHOLD).toBe(1000)
  })

  test('previews label or normalized text with truncation', () => {
    expect(composerTextArtifactPreview(createComposerTextArtifactDraft('  hello\n  world '))).toBe('hello world')
    expect(composerTextArtifactPreview(createComposerTextArtifactDraft('long body', 'My paste'))).toBe('My paste')
    const preview = composerTextArtifactPreview(createComposerTextArtifactDraft('x'.repeat(120)))
    expect(preview.length).toBe(80)
  })
})
