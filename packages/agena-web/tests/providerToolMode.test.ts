import assert from 'node:assert/strict'
import test from 'node:test'

import { effectiveProviderToolMode } from '../src/lib/providerToolMode'

test('missing, unknown and incomplete metadata assume tools without mutating the draft', () => {
  for (const config of [{}, { agena_tools: {} }, { features: [] }, { features: ['streaming'] }]) {
    for (const support of [undefined, 'unknown', 'supported']) {
      const before = structuredClone(config)
      assert.equal(effectiveProviderToolMode(config, support), 'provider_protocol')
      assert.deepEqual(config, before)
    }
  }
})

test('only an explicit negative capability disables inherited tools', () => {
  assert.equal(effectiveProviderToolMode({}, 'unsupported'), 'disabled')
  assert.equal(effectiveProviderToolMode({ features: { unsupported: ['tool_calling'] } }), 'disabled')
  assert.equal(effectiveProviderToolMode({ features: ['tool_calling'] }, 'unsupported'), 'provider_protocol')
  assert.equal(
    effectiveProviderToolMode({ features: { supported: ['tool_calling'] } }, 'unsupported'),
    'provider_protocol',
  )
  assert.equal(effectiveProviderToolMode({ features: { unsupported: ['streaming'] } }), 'provider_protocol')
})

test('user-selected tool modes override all capability declarations', () => {
  assert.equal(
    effectiveProviderToolMode({ agena_tools: { mode: 'disabled' }, features: ['tool_calling'] }, 'supported'),
    'disabled',
  )
  assert.equal(
    effectiveProviderToolMode(
      { agena_tools: { mode: 'provider_protocol' }, features: { unsupported: ['tool_calling'] } },
      'unsupported',
    ),
    'provider_protocol',
  )
})
