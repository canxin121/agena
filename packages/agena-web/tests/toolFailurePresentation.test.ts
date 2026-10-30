import { describe, expect, test } from 'bun:test'
import { i18n } from '../src/i18n'
import { toolFailurePresentation } from '../src/lib/toolFailures'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'
import { operationPresentation } from '../src/pages/chat/transcriptPartPresentation'

const publicProblem = {
  id: '0f9ad8bb-c04c-4e99-a82c-9fabf4411b13',
  code: 'tool.invalid_input',
  category: 'invalid_input',
  responsibility: 'caller',
  retry: 'correct_input',
  recovery: 'none',
  impact: 'request_rejected',
  user: {
    key: 'tool.invalid_input',
    fallback: 'The required file_path argument is missing.',
    detail_key: 'tool.invalid_input.detail',
  },
}

function failedTool(name: string, error: unknown = null, state = 'failed') {
  return projectLocalPart(
    { id: '1145', agenaKind: 'tool_call', partState: state, agenaContent: { name, error } },
    'assistant',
  )
}

describe('public tool failure presentation', () => {
  test.each([
    ['agena.provider', 'The operation failed. Review the logs for details.'],
    ['shell.exec', "$: Additional properties are not allowed ('cwd', 'timeout_seconds' were unexpected)"],
    ['fs.read', '$: "file_path" is a required property'],
    ['fs.read', 'read target does not exist: artifacts/provider-tools/result.json'],
    ['fs.read_media', "$: Additional properties are not allowed ('file_path' was unexpected)"],
    ['fs.read_media', 'fs.read_media requires an active conversation model route'],
    ['chatgpt.image_understanding', '$: "inputs" is a required property'],
    ['chatgpt.image_understanding', 'cloud media analysis returned HTTP 400 Bad Request; inspect status before retrying'],
  ])('%s exposes the supplied public reason even when Output is null', (name, message) => {
    const problem = { ...publicProblem, user: { ...publicProblem.user, fallback: message } }
    const display = failedTool(name, { failure: problem })
    const original = structuredClone(display.source)
    for (const expanded of [false, true]) {
      const operation = operationPresentation(display, { output: null, metadata: {}, presentation: {} }, expanded)
      expect(operation.error).toBe(message)
      expect(operation.errorDetails).toEqual(problem)
      expect(operation.rawOutput).toBeNull()
    }
    expect(display.source).toEqual(original)
  })

  test('Failure, problem and direct UserProblem envelopes expose only public fields', () => {
    const value = {
      ...publicProblem,
      model: { text: 'PRIVATE_MODEL_SENTINEL', policy: 'PRIVATE_POLICY_SENTINEL' },
      diagnostic: { source: 'PRIVATE_DIAGNOSTIC_SENTINEL' },
      cause: 'PRIVATE_CAUSE_SENTINEL',
      user: { ...publicProblem.user, diagnostic: 'PRIVATE_USER_EXTENSION_SENTINEL' },
    }
    for (const error of [{ failure: value }, { problem: value }, value]) {
      const result = toolFailurePresentation(error)
      expect(result).toEqual({ message: publicProblem.user.fallback, details: publicProblem })
      expect(JSON.stringify(result)).not.toContain('PRIVATE_')
    }
  })

  test('failed states with absent or malformed public reasons explain the missing information', () => {
    for (const error of [null, undefined, {}, { failure: null }, { failure: { model: { text: 'PRIVATE' } } }]) {
      for (const state of ['failed', 'error']) {
        const operation = operationPresentation(failedTool('fs.read', error, state))
        expect(operation.error).toBe(i18n.global.t('chat.toolDetails.errorUnavailable'))
        expect(JSON.stringify(operation.errorDetails)).not.toContain('PRIVATE')
      }
    }
  })

  test('successful, running, cancelled and declined tools do not invent failures', () => {
    for (const state of ['completed', 'running', 'in_progress', 'pending', 'cancelled', 'user_declined']) {
      const operation = operationPresentation(failedTool('shell.exec', null, state))
      expect(operation.error).toBe('')
      expect(operation.errorDetails).toBeNull()
    }
  })

  test('legacy public messages stay readable without copying private extensions', () => {
    expect(toolFailurePresentation({ message: 'Command failed.', diagnostic: 'PRIVATE' })).toEqual({
      message: 'Command failed.',
      details: { message: 'Command failed.' },
    })
    expect(toolFailurePresentation({ failure: { message: 'File not found.' } })).toEqual({
      message: 'File not found.',
      details: { message: 'File not found.' },
    })
  })

  test('the main error is not repeated as a body block, but distinct stderr and partial media remain', () => {
    const display = failedTool('shell.exec', { failure: publicProblem })
    display.source.agenaPresentation = {
      blocks: [
        { type: 'text', id: 'failure', text: publicProblem.user.fallback },
        { type: 'log', id: 'stderr', stream: 'stderr', text: 'One input could not be processed.' },
        { type: 'media', id: 'partial', artifact: { uri: 'file:///generated/partial.png', mime: 'image/png' } },
      ],
    }
    const operation = operationPresentation(display)
    expect(operation.error).toBe(publicProblem.user.fallback)
    expect(operation.presentationBlocks.map((block) => block.id)).toEqual(['stderr', 'partial'])
  })
})
