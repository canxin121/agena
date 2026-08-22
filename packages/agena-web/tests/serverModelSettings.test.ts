import assert from 'node:assert/strict'
import test from 'node:test'

import {
  approvalModelFromSettingsResponse,
  buildApprovalModelSettingsPatch,
  buildDefaultModelSettingsPatch,
  normalizeServerModelIdentity,
  sameServerModelIdentity,
} from '../src/lib/serverModelSettings'

test('global default model patch writes only providers.default_selection', () => {
  assert.deepEqual(
    buildDefaultModelSettingsPatch(
      { provider: 'openai', adapter: 'responses', model: 'gpt-5' },
      { thinkingMode: 'high', speedMode: 'fast', verbosity: 'compact', parallelToolCalls: true },
    ),
    {
      path: 'providers',
      changes: {
        default_selection: {
          provider: 'openai',
          adapter: 'responses',
          model: 'gpt-5',
          thinking_mode: 'high',
          speed_mode: 'fast',
          verbosity: 'compact',
          parallel_tool_calls: true,
        },
      },
      dry_run: false,
      validate: true,
      reload: true,
    },
  )
  assert.equal('default' in buildDefaultModelSettingsPatch({ provider: 'openai', model: 'gpt-5' }).changes, false)
})

test('approval model patch uses the permission-specific identity field names', () => {
  assert.deepEqual(
    buildApprovalModelSettingsPatch(
      { provider: 'anthropic', adapter: 'messages', model: 'claude-sonnet' },
      { thinkingMode: 'high' },
    ),
    {
      path: 'permission',
      changes: {
        approval_model: {
          provider_id: 'anthropic',
          adapter_id: 'messages',
          model_id: 'claude-sonnet',
          thinking_mode: 'high',
        },
      },
      dry_run: false,
      validate: true,
      reload: true,
    },
  )
  assert.deepEqual(buildApprovalModelSettingsPatch(null).changes, { approval_model: null })
})

test('settings read-back normalizes approval model identity and modes', () => {
  assert.deepEqual(
    approvalModelFromSettingsResponse({
      source: 'effective',
      path: 'permission.approval_model',
      value: {
        provider_id: 'openai',
        adapter_id: 'responses',
        model_id: 'gpt-5',
        thinking_mode: 'high',
        speed_mode: 'fast',
        verbosity: 'compact',
        parallel_tool_calls: true,
      },
    }),
    {
      identity: { provider: 'openai', adapter: 'responses', model: 'gpt-5' },
      modes: {
        thinkingMode: 'high',
        speedMode: 'fast',
        verbosity: 'compact',
        parallelToolCalls: true,
      },
    },
  )
  assert.equal(approvalModelFromSettingsResponse({ value: null }), null)
})

test('model identity comparison includes adapter identity', () => {
  const resource = normalizeServerModelIdentity({
    provider_id: 'openai',
    adapter_id: 'responses',
    model_id: 'gpt-5',
  })
  assert.equal(sameServerModelIdentity(resource, { provider: 'openai', adapter: 'responses', model: 'gpt-5' }), true)
  assert.equal(
    sameServerModelIdentity(resource, { provider: 'openai', adapter: 'chat-completions', model: 'gpt-5' }),
    false,
  )
})
