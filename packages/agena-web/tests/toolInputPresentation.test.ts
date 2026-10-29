import { describe, expect, test } from 'bun:test'
import { normalizeAgenaPart } from '../src/stores/chat/api'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'
import { operationPresentation, structuredValueMarkdown } from '../src/pages/chat/transcriptPartPresentation'
import { renderMarkdown } from '../src/lib/markdown'
import type { JsonObject, JsonValue } from '../src/types/json'

const input = {
  command: "printf '%s\\n' 'hello'\npwd",
  workdir: '/workspace',
  description: '{"literal":"keep this JSON text"}',
  reads: ['.'],
  writes: [],
  network: [],
}
const commandMarkdown = `\`\`\`sh\n${input.command}\n\`\`\`\n`

function renderedCodeBlocks(markdown: string): string[] {
  const entities: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"' }
  return [...renderMarkdown(markdown).matchAll(/<pre\b[^>]*>\s*<code\b[^>]*>([\s\S]*?)<\/code>\s*<\/pre>/g)]
    .map((match) => match[1]!
      .replace(/<[^>]*>/g, '')
      .replace(/&(amp|lt|gt|quot|#(?:x[0-9a-f]+|\d+));/gi, (entity, name: string) => {
        if (!name.startsWith('#')) return entities[name.toLowerCase()] ?? entity
        const hex = name[1]?.toLowerCase() === 'x'
        return String.fromCodePoint(Number.parseInt(name.slice(hex ? 2 : 1), hex ? 16 : 10))
      }))
}

function providerCall(wireInput: string): JsonObject {
  return {
    function: 'tools_call',
    arguments: {
      fields: [
        { name: 'tool', value: { kind: 'text', value: 'shell.exec' } },
        { name: 'input', value: { kind: 'text', value: wireInput } },
      ],
    },
  }
}

function projectTool(content: JsonObject, state = 'running') {
  const normalized = normalizeAgenaPart('42', '7', '11', {
    part_id: 42,
    run_id: 11,
    kind: 'tool_call',
    role: 'assistant',
    state,
    content: { name: 'shell.exec', call_id: 42, ...content },
    sections: [],
    presentation: { title: 'shell.exec', summary: '', blocks: [] },
  })
  if (!normalized) throw new Error('tool_call normalization returned no part')
  return { normalized, display: projectLocalPart(normalized, 'assistant') }
}

describe('tool input wire forms reach the transcript', () => {
  for (const [form, wireInput] of [
    ['object', input],
    ['JSON string', JSON.stringify(input)],
  ] as const) {
    test(`${form} input survives normalization, display and copy`, () => {
      const { normalized, display } = projectTool({ input: wireInput })
      expect(normalized.state).toMatchObject({ input })
      expect(display.kind).toBe('operation')

      const operation = operationPresentation(display)
      expect(operation.input).toEqual(input)
      expect(operation.commandMarkdown).toBe(commandMarkdown)
      expect(renderedCodeBlocks(operation.inputMarkdown)).toContain(`${input.command}\n`)
      expect(operation.inputMarkdown).toContain(input.description)
      expect(display.copyText).toContain(`Input\n${JSON.stringify(input, null, 2)}`)

      // Decoding input must not parse literal JSON text in an argument or
      // rewrite the source envelope while deriving a display value.
      expect(normalized.agenaContent).toMatchObject({ input: wireInput })
      const collapsed = operationPresentation(display, {}, false)
      expect(collapsed.commandMarkdown).toBe('')
      expect(collapsed.inputMarkdown).toBe('')
    })
  }

  test('omitted input stays lazy even when the provider envelope contains arguments', () => {
    const toolApiCall = providerCall(JSON.stringify(input))
    const { normalized, display } = projectTool({ tool_api_call: toolApiCall })

    expect(normalized.agenaContent).not.toHaveProperty('input')
    expect(operationPresentation(display).input).toBeNull()
    expect(operationPresentation(display).commandMarkdown).toBe('')
    expect(display.copyText).not.toContain('Input\n')

    for (const wireInput of [input, JSON.stringify(input)]) {
      const operation = operationPresentation(display, { input: wireInput })
      expect(operation.input).toEqual(input)
      expect(operation.commandMarkdown).toBe(commandMarkdown)
      expect(renderedCodeBlocks(operation.inputMarkdown)).toContain(`${input.command}\n`)
    }

    // A loaded child section is local presentation state, not a mutation of
    // the omitted transcript snapshot or its provider envelope.
    expect(normalized.agenaContent).not.toHaveProperty('input')
    expect(normalized.agenaContent).toMatchObject({ tool_api_call: toolApiCall })
    expect(display.copyText).not.toContain('Input\n')
  })

  test('the fetched input section replaces a stale snapshot for either wire form', () => {
    const staleInput = { command: 'echo stale snapshot' }
    const { display } = projectTool({ input: staleInput }, 'completed')

    for (const wireInput of [input, JSON.stringify(input)]) {
      const operation = operationPresentation(display, { input: wireInput })
      expect(operation.input).toEqual(input)
      expect(operation.commandMarkdown).toBe(commandMarkdown)
      expect(operation.inputMarkdown).not.toContain(staleInput.command)
    }

    for (const emptyInput of [{}, '{}', null] satisfies JsonValue[]) {
      const operation = operationPresentation(display, { input: emptyInput })
      expect(operation.input).toBeNull()
      expect(operation.commandMarkdown).toBe('')
      expect(operation.inputMarkdown).toBe('')
    }
  })

  test('canonical input takes precedence over raw provider arguments, including an empty object', () => {
    const toolApiCall = providerCall('{"command":"echo raw provider arguments"}')
    for (const wireInput of [input, JSON.stringify(input)]) {
      const { normalized, display } = projectTool({ input: wireInput, tool_api_call: toolApiCall })
      expect(normalized.state).toMatchObject({ input })
      expect(operationPresentation(display).commandMarkdown).toBe(commandMarkdown)
      expect(display.copyText).not.toContain('echo raw provider arguments')
    }
    for (const emptyInput of [{}, '{}']) {
      const { normalized, display } = projectTool({ input: emptyInput, tool_api_call: toolApiCall })
      expect(normalized.state).toMatchObject({ input: {} })
      expect(operationPresentation(display).input).toBeNull()
      expect(operationPresentation(display).commandMarkdown).toBe('')
      expect(display.copyText).not.toContain('Input\n')
    }
  })

  test('real Markdown rendering preserves multiline commands and nested code fences verbatim', () => {
    const command = "printf '%s\\n' '中文 🌱'\ncat <<'EOF'\n``` & <tag> \"quoted\" `literal` \\path\nEOF"
    const markdown = structuredValueMarkdown({ command, nested: { scripts: [command] } })
    expect(renderedCodeBlocks(markdown)).toEqual([`${command}\n`, `${command}\n`])
  })
})
