import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  MAX_PROMPT_FRAGMENT_BYTES,
  emptyPromptLayer,
  emptyPromptScopeConfig,
  isValidPromptFragmentId,
  nextPromptFragmentId,
  normalizePromptLayer,
  normalizePromptScopeConfig,
  promptFragmentBodyBytes,
  promptLayerAddFragment,
  promptLayerAddOrder,
  promptLayerHasErrors,
  promptLayerIsEmpty,
  promptLayerMoveOrder,
  promptLayerRemoveFragment,
  promptLayerSetDisabled,
  promptLayerUpdateFragment,
  promptLayersEqual,
  promptScopeConfigRemoveFragment,
  promptScopeConfigUpdateFragment,
  promptScopeConfigIsEmpty,
  promptScopeConfigWithLayer,
  validatePromptLayer,
  type PromptFragment,
  type PromptLayer,
  type PromptScopeConfig,
} from '../src/lib/promptLayers'

function fragment(id: string, body = 'Body'): PromptFragment {
  return { id, title: id, body, when: { models: [], requires_tools: null } }
}

function layer(overrides: Partial<PromptLayer> = {}): PromptLayer {
  return { ...emptyPromptLayer(), ...overrides }
}

function scopeConfig(overrides: Partial<PromptScopeConfig> = {}): PromptScopeConfig {
  return { ...emptyPromptScopeConfig(), ...overrides }
}

test('normalization is lossless for supported fields and drops unknown ones', () => {
  assert.deepEqual(
    normalizePromptLayer({
      mode: 'replace',
      order: ['one', 2, 'two'],
      disabled: ['legacy'],
      inline: [
        {
          id: 'one',
          title: 'One',
          body: 'Body',
          when: { models: [' anthropic/* ', ''], requires_tools: true, ignored: true },
          ignored: true,
        },
      ],
      ignored: true,
    }),
    {
      mode: 'replace',
      order: ['one', 'two'],
      disabled: ['legacy'],
      inline: [
        {
          id: 'one',
          title: 'One',
          body: 'Body',
          when: { models: ['anthropic/*'], requires_tools: true },
        },
      ],
    },
  )
})

test('fragment ids mirror the Rust validation exactly', () => {
  for (const id of ['a', '_hidden', '-leading', '9', 'a.b_c-d', 'x'.repeat(64)]) {
    assert.equal(isValidPromptFragmentId(id), true, id)
  }
  for (const id of ['', '.hidden', 'Uppercase', 'with space', 'x'.repeat(65), 'é']) {
    assert.equal(isValidPromptFragmentId(id), false, id)
  }
})

test('structural validation catches every editable layer conflict', () => {
  const invalid = layer({
    order: ['one', 'one', 'inherited'],
    disabled: ['blocked', 'blocked', 'inherited'],
    inline: [
      fragment('one', ' '.repeat(1)),
      fragment('one'),
      fragment('.bad'),
      fragment('large', 'x'.repeat(MAX_PROMPT_FRAGMENT_BYTES + 1)),
      fragment('unordered'),
    ],
  })
  const issues = validatePromptLayer(invalid, true)
  const codes = new Set(issues.map((issue) => issue.code))
  for (const code of [
    'duplicate_order',
    'duplicate_disabled',
    'duplicate_inline',
    'order_disabled_conflict',
    'invalid_id',
    'fragment_too_large',
    'empty_body',
    'inline_not_in_order',
    'external_order_fragment',
  ]) {
    assert.equal(codes.has(code), true, code)
  }
  assert.equal(promptLayerHasErrors(issues), true)
  assert.equal(promptLayerHasErrors(validatePromptLayer(layer())), false)
  const duplicateDisabled = validatePromptLayer(layer({ disabled: ['inherited', 'inherited'] }), true)
  assert.deepEqual(
    duplicateDisabled.map((issue) => [issue.code, issue.level]),
    [['duplicate_disabled', 'warning']],
  )
  assert.equal(promptLayerHasErrors(duplicateDisabled), false)
  assert.equal(
    promptLayerHasErrors(validatePromptLayer(layer({ disabled: ['one'], inline: [fragment('one')] }), true)),
    true,
  )
})

test('scope edits preserve model route overlays', () => {
  const config = normalizePromptScopeConfig({
    order: ['base'],
    models: [{ match: 'anthropic/*', order: ['opus'] }],
  })
  assert.deepEqual(config.models, [{ match: 'anthropic/*', mode: 'append', order: ['opus'], disabled: [], inline: [] }])
  const updated = promptScopeConfigWithLayer(config, layer({ order: ['next'], mode: 'replace' }))
  assert.deepEqual(updated.models, config.models)
  assert.equal(updated.mode, 'replace')
  assert.deepEqual(updated.order, ['next'])
  assert.deepEqual(promptScopeConfigWithLayer(config, emptyPromptLayer()).models, config.models)
  assert.equal(promptScopeConfigIsEmpty(emptyPromptScopeConfig()), true)
})

test('renaming a fragment syncs model layer order and disabled references', () => {
  const config = scopeConfig({
    order: ['old', 'other'],
    disabled: ['old'],
    inline: [fragment('old'), fragment('other')],
    models: [
      normalizePromptScopeConfig({
        models: [{ match: 'anthropic/*', order: ['old', 'other'], disabled: ['old'] }],
      }).models[0],
      normalizePromptScopeConfig({
        models: [{ match: 'openai/*', order: ['other'], disabled: [] }],
      }).models[0],
    ],
  })

  const updated = promptScopeConfigUpdateFragment(config, 0, fragment('new'))

  assert.equal(updated.inline[0].id, 'new')
  assert.deepEqual(updated.order, ['new', 'other'])
  assert.deepEqual(updated.disabled, ['new'])
  assert.deepEqual(updated.models[0].order, ['new', 'other'])
  assert.deepEqual(updated.models[0].disabled, ['new'])
  assert.deepEqual(updated.models[1].order, ['other'])
})

test('removing a fragment syncs model layer order and disabled references', () => {
  const config = scopeConfig({
    order: ['old', 'other'],
    disabled: ['old', 'inherited'],
    inline: [fragment('old'), fragment('other')],
    models: [
      normalizePromptScopeConfig({
        models: [{ match: 'anthropic/*', order: ['old', 'other'], disabled: ['old', 'inherited'] }],
      }).models[0],
    ],
  })

  const updated = promptScopeConfigRemoveFragment(config, 'old')

  assert.deepEqual(
    updated.inline.map((entry) => entry.id),
    ['other'],
  )
  assert.deepEqual(updated.order, ['other'])
  assert.deepEqual(updated.disabled, ['inherited'])
  assert.deepEqual(updated.models[0].order, ['other'])
  assert.deepEqual(updated.models[0].disabled, ['inherited'])
})

test('model layers that own the fragment inline keep their own references', () => {
  const config = normalizePromptScopeConfig({
    order: ['old', 'other'],
    inline: [fragment('old'), fragment('other')],
    models: [
      { match: 'anthropic/*', order: ['old', 'other'], disabled: ['old'] },
      { match: 'local/*', inline: [fragment('old')], order: ['old'], disabled: ['old'] },
    ],
  })
  const shadowed = config.models[1]

  const renamed = promptScopeConfigUpdateFragment(config, 0, fragment('renamed'))
  assert.deepEqual(renamed.models[0].order, ['renamed', 'other'])
  assert.deepEqual(renamed.models[0].disabled, ['renamed'])
  assert.deepEqual(renamed.models[1], shadowed)

  const removed = promptScopeConfigRemoveFragment(config, 'old')
  assert.deepEqual(removed.models[0].order, ['other'])
  assert.deepEqual(removed.models[0].disabled, [])
  assert.deepEqual(removed.models[1], shadowed)
})

test('fragment edits keep order and disabled references in sync', () => {
  const original = layer({ order: ['one'], inline: [fragment('one')] })
  const renamed = promptLayerUpdateFragment(original, 0, fragment('renamed'))
  assert.deepEqual(renamed.order, ['renamed'])
  assert.equal(renamed.inline[0].id, 'renamed')

  const appended = promptLayerAddFragment(emptyPromptLayer(), fragment('next'))
  assert.deepEqual(appended.order, ['next'])
  assert.deepEqual(
    appended.inline.map((entry) => entry.id),
    ['next'],
  )
  assert.deepEqual(promptLayerRemoveFragment(appended, 'next'), emptyPromptLayer())
})

test('order movement and inherited disabling stay deterministic', () => {
  const original = layer({ order: ['one', 'two', 'three'] })
  assert.deepEqual(promptLayerMoveOrder(original, 'two', -1).order, ['two', 'one', 'three'])
  assert.deepEqual(promptLayerMoveOrder(original, 'two', 1).order, ['one', 'three', 'two'])
  assert.deepEqual(promptLayerMoveOrder(original, 'missing', 1), original)

  const inherited = promptLayerSetDisabled(original, 'inherited', true)
  assert.deepEqual(inherited.disabled, ['inherited'])
  assert.deepEqual(promptLayerSetDisabled(inherited, 'inherited', false).disabled, [])

  const inline = layer({ order: ['one'], inline: [fragment('one')] })
  const disabledInline = promptLayerSetDisabled(inline, 'one', true)
  assert.deepEqual(disabledInline.order, [])
  assert.deepEqual(disabledInline.disabled, [])
  assert.deepEqual(promptLayerAddOrder(disabledInline, 'one').order, ['one'])
})

test('new ids, equality, emptiness, and UTF-8 byte limits are stable', () => {
  const existing = layer({ order: ['fragment-1'], inline: [fragment('fragment-1')] })
  assert.equal(nextPromptFragmentId(existing), 'fragment-2')
  assert.equal(nextPromptFragmentId(layer({ order: ['fragment-1'] })), 'fragment-2')
  const shadowed = promptLayerAddFragment(layer({ order: ['fragment-1'] }), fragment('fragment-1'))
  assert.deepEqual(shadowed.order, ['fragment-1'])
  assert.equal(promptLayersEqual(emptyPromptLayer(), normalizePromptLayer({})), true)
  assert.equal(promptLayersEqual(layer({ mode: 'replace' }), emptyPromptLayer()), false)
  assert.equal(promptLayerIsEmpty(emptyPromptLayer()), true)
  assert.equal(promptLayerIsEmpty(layer({ mode: 'replace' })), false)
  assert.equal(promptFragmentBodyBytes('中'), 3)
  assert.equal(promptFragmentBodyBytes('a中'), 4)
})
