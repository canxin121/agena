import assert from 'node:assert/strict'
import test from 'node:test'

import {
  defaultValueForSchema,
  deriveConfigOverride,
  materializeConfigValue,
  schemaBranches,
  schemaMatchesValue,
} from '../src/components/settings/plugins/pluginConfigSchema'

test('plugin config overrides preserve explicit removal of materialized defaults', () => {
  const defaults = { enabled: true, nested: { mode: 'safe' } }
  const effective = { nested: {} }
  assert.deepEqual(deriveConfigOverride(defaults, effective), {
    enabled: null,
    nested: { mode: null },
  })
})

test('plugin config materialization merges minimal overrides into schema defaults', () => {
  const schema = {
    type: 'object',
    properties: {
      enabled: { type: 'boolean', default: true },
      limit: { type: 'integer', default: 10 },
    },
  }
  assert.deepEqual(defaultValueForSchema(schema), { enabled: true, limit: 10 })
  assert.deepEqual(materializeConfigValue(schema, { limit: 20 }), { enabled: true, limit: 20 })
})

test('union branch matching follows object const discriminators', () => {
  const schema = {
    oneOf: [
      {
        title: 'Static',
        type: 'object',
        required: ['kind'],
        properties: { kind: { const: 'static' }, path: { type: 'string' } },
      },
      {
        title: 'HTTP',
        type: 'object',
        required: ['kind'],
        properties: { kind: { const: 'http' }, url: { type: 'string' } },
      },
    ],
  }
  const branches = schemaBranches(schema, schema)
  assert.equal(schemaMatchesValue(branches[0]!, { kind: 'http', url: 'https://example.com' }, schema), false)
  assert.equal(schemaMatchesValue(branches[1]!, { kind: 'http', url: 'https://example.com' }, schema), true)
})
