import test from 'node:test'
import assert from 'node:assert/strict'

import {
  buildCreateSessionRequest,
  buildMessageRequestBody,
  buildRunRequestBody,
  interactivePresentationPath,
  messageErrorFromAgenaPart,
  normalizeAgenaPart,
} from '../src/stores/chat/api'

test('session create request includes the backend-required workspace and title', () => {
  assert.deepEqual(buildCreateSessionRequest({ workspaceId: 42, title: ' New session ', parentId: 7 }), {
    workspace_id: 42,
    title: 'New session',
    parent_id: 7,
  })
})

test('message request flattens Agena RunOptions beside the document', () => {
  const body = buildMessageRequestBody({
    document: [{ type: 'text', text: 'hello' }],
    model: {
      provider_id: 'openai',
      adapter_id: 'responses',
      model_id: 'gpt-5',
    },
    thinking_mode: 'high',
    speed_mode: 'fast',
    verbosity: 'medium',
    parallel_tool_calls: true,
  })

  assert.deepEqual(body, {
    document: [{ type: 'text', text: 'hello' }],
    model: {
      provider_id: 'openai',
      adapter_id: 'responses',
      model_id: 'gpt-5',
    },
    thinking_mode: 'high',
    speed_mode: 'fast',
    verbosity: 'medium',
    parallel_tool_calls: true,
  })
  assert.equal(Object.prototype.hasOwnProperty.call(body, 'options'), false)
})

test('continue and compact request options use the same flattened whitelist', () => {
  const body = buildRunRequestBody({
    model: { provider_id: 'anthropic', model_id: 'claude-sonnet' },
    thinking_mode: 'low',
    temperature: 0.2,
    max_output_tokens: 2048,
  })

  assert.deepEqual(body, {
    model: { provider_id: 'anthropic', model_id: 'claude-sonnet' },
    thinking_mode: 'low',
    temperature: 0.2,
    max_output_tokens: 2048,
  })
  assert.equal(Object.prototype.hasOwnProperty.call(body, 'options'), false)
  assert.equal(Object.prototype.hasOwnProperty.call(body, 'agent'), false)
  assert.equal(Object.prototype.hasOwnProperty.call(body, 'variant'), false)
})

test('interactive presentation acknowledgement is scoped to its session', () => {
  assert.equal(
    interactivePresentationPath('12', 'permission/request 1'),
    '/api/v1/sessions/12/interactive/permission%2Frequest%201/present',
  )
})

test('durable Agena error parts expose the user-facing problem envelope', () => {
  assert.deepEqual(
    messageErrorFromAgenaPart({
      part_id: 9,
      kind: 'error',
      role: 'assistant',
      state: 'failed',
      content: {
        category: 'provider',
        message: 'diagnostic text',
        problem: {
          code: 'provider.unavailable',
          category: 'configuration',
          user: { fallback: 'The configured provider is unavailable.' },
        },
      },
    }),
    {
      name: 'AgenaError',
      type: 'configuration',
      message: 'The configured provider is unavailable.',
      code: 'provider.unavailable',
      classification: 'configuration',
      problem: {
        code: 'provider.unavailable',
        category: 'configuration',
        user: { fallback: 'The configured provider is unavailable.' },
      },
    },
  )
})

test('reasoning parts concatenate Agena streaming fragments without adding lines', () => {
  const part = normalizeAgenaPart('10', '7', '9', {
    part_id: 10,
    kind: 'think',
    role: 'assistant',
    state: 'completed',
    content: { summary: ['The', ' model', ' is', ' thinking.'] },
  })

  assert.equal(part?.type, 'reasoning')
  assert.equal(part?.text, 'The model is thinking.')
})

test('tool-call parts read canonical operation invocation and result fields', () => {
  const part = normalizeAgenaPart('11', '7', '10', {
    part_id: 11,
    kind: 'tool_call',
    role: 'assistant',
    state: 'completed',
    content: {
      name: 'agena.fs.read',
      input: { file_path: 'README.md' },
      state: 'completed',
      output: { payload: { text: 'README contents', bytes: 42 } },
      metadata: { cache: true },
    },
    presentation: {
      title: 'Read README.md',
      summary: 'README contents',
      blocks: [{ type: 'markdown', text: 'README contents' }],
    },
  })

  assert.equal(part?.type, 'tool')
  assert.equal(part?.tool, 'agena.fs.read')
  assert.deepEqual(part?.state, {
    status: 'completed',
    input: { file_path: 'README.md' },
    output: 'README contents',
    title: 'Read README.md',
    metadata: { cache: true },
  })
  assert.equal(part?.agenaKind, 'tool_call')
  assert.equal(part?.agenaRole, 'assistant')
  assert.equal(part?.partState, 'completed')
  assert.deepEqual(part?.agenaContent, {
    name: 'agena.fs.read',
    input: { file_path: 'README.md' },
    state: 'completed',
    output: { payload: { text: 'README contents', bytes: 42 } },
    metadata: { cache: true },
  })
  assert.deepEqual(part?.agenaPresentation, {
    title: 'Read README.md',
    summary: 'README contents',
    blocks: [{ type: 'markdown', text: 'README contents' }],
  })
})

test('tool-call parts retain structured results when no display text exists', () => {
  const part = normalizeAgenaPart('12', '7', '10', {
    part_id: 12,
    kind: 'tool_call',
    role: 'assistant',
    state: 'completed',
    content: {
      name: 'agena.repo.status',
      input: {},
      state: 'completed',
      output: { payload: { clean: true } },
    },
    presentation: { title: 'Repository status', summary: 'Working tree is clean', blocks: [] },
  })
  assert.deepEqual(part?.state, {
    status: 'completed',
    input: {},
    output: 'Working tree is clean',
    title: 'Repository status',
  })
})

test('tool-call parts honor Agena result lifecycle and presentation summary', () => {
  const part = normalizeAgenaPart('14', '7', '10', {
    part_id: 14,
    kind: 'tool_call',
    role: 'assistant',
    state: 'completed',
    content: {
      name: 'fs.read',
      input: { file_path: 'missing.txt' },
      state: 'capability_unavailable',
      output: { payload: { text: 'This runtime cannot read files.' } },
    },
    presentation: {
      title: 'File capability unavailable',
      summary: 'This runtime cannot read files.',
      blocks: [{ type: 'text', text: 'This runtime cannot read files.' }],
    },
  })

  assert.deepEqual(part?.state, {
    status: 'error',
    input: { file_path: 'missing.txt' },
    output: 'This runtime cannot read files.',
    title: 'File capability unavailable',
  })
})

test('file references retain Agena URL attachment sources', () => {
  const part = normalizeAgenaPart('15', '7', '10', {
    part_id: 15,
    kind: 'file_ref',
    role: 'assistant',
    state: 'completed',
    content: {
      name: 'result.png',
      mime: 'image/png',
      source: { source: 'url', url: 'https://example.test/result.png' },
    },
  })

  assert.equal(part?.type, 'file')
  assert.equal(part?.filename, 'result.png')
  assert.equal(part?.mime, 'image/png')
  assert.equal(part?.url, 'https://example.test/result.png')
})

test('missing or unknown wire state is not invented as completed, pending, or failed', () => {
  const textPart = normalizeAgenaPart('90', '7', '89', {
    part_id: 90,
    kind: 'text',
    role: 'assistant',
    content: { text: 'server omitted state' },
  })
  assert.equal(textPart?.partState, undefined)

  const toolPart = normalizeAgenaPart('91', '7', '89', {
    part_id: 91,
    kind: 'tool_call',
    role: 'assistant',
    state: 'future_state',
    content: { name: 'future.tool', input: {} },
  })
  assert.deepEqual(toolPart?.state, { input: {} })
  assert.equal(toolPart?.partState, 'future_state')
})
