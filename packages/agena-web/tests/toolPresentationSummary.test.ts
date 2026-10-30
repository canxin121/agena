import { describe, expect, test } from 'bun:test'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'
import { operationPresentation } from '../src/pages/chat/transcriptPartPresentation'

const caption = 'A rose in a vase, lit from the left'
const presentation = {
  title: 'Generate image · 1 image',
  summary: '',
  blocks: [
    { type: 'markdown', id: 'text', text: caption },
    { type: 'media', id: 'media', artifact: { uri: 'file:///generated/rose.png', mime: 'image/png' } },
  ],
}
const source = {
  id: '557',
  agenaKind: 'tool_call',
  partState: 'completed',
  agenaSummary: 'A stale copy of the image prompt',
  agenaContent: { name: 'image_generation', input: { prompt: 'Draw a rose' } },
}

describe('authoritative tool presentation summaries', () => {
  test('an empty projected summary does not revive the stored prompt in collapsed or expanded views', () => {
    const display = projectLocalPart({ ...source, agenaPresentation: presentation }, 'assistant')
    expect(display.title).toBe(presentation.title)
    expect(display.summary).toBe('')
    for (const details of [false, true]) {
      const operation = operationPresentation(display, {}, details)
      expect(operation.title).toBe(presentation.title)
      expect(operation.summary).toBe('')
      if (details) {
        expect(operation.presentationBlocks.filter((block) => block.text === caption)).toHaveLength(1)
        expect(operation.presentationBlocks.filter((block) => block.type === 'media')).toHaveLength(1)
      }
    }
    expect(display.source.agenaSummary).toBe(source.agenaSummary)
  })

  test('a lazy presentation read can intentionally replace a stale snapshot summary with empty', () => {
    const display = projectLocalPart(source, 'assistant')
    expect(display.summary).toBe(source.agenaSummary)
    expect(operationPresentation(display).summary).toBe(source.agenaSummary)
    const operation = operationPresentation(display, { presentation })
    expect(operation.title).toBe(presentation.title)
    expect(operation.summary).toBe('')
    expect(operation.presentationBlocks).toHaveLength(2)
  })

  test('distinct projected summaries remain visible and missing summaries retain the legacy fallback', () => {
    const summary = 'The provider adjusted the requested dimensions'
    const display = projectLocalPart({ ...source, agenaPresentation: { ...presentation, summary } }, 'assistant')
    expect(display.summary).toBe(summary)
    expect(operationPresentation(display).summary).toBe(summary)

    const legacy = projectLocalPart({ ...source, agenaPresentation: { blocks: presentation.blocks } }, 'assistant')
    expect(legacy.summary).toBe(source.agenaSummary)
    expect(operationPresentation(legacy).summary).toBe(source.agenaSummary)
  })
})
