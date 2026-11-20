import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import { hasRuntimeLayerValue, runtimeSettingBaseValue, runtimeSettingLayer } from '../src/lib/runtimeSettingLayer'
import type { RuntimeSettingsReadBundle } from '../src/lib/runtimeSettings'

function bundle(
  global?: unknown,
  workspace?: unknown,
  effective?: unknown,
  parent?: unknown,
): RuntimeSettingsReadBundle {
  return {
    global: { config_found: true, value: global as any, inherited_value: true },
    workspace: { config_found: true, value: workspace as any, inherited_value: (parent ?? global ?? true) as any },
    file: { value: global as any },
    effective: { value: (effective ?? workspace ?? global ?? true) as any },
  }
}

describe('selected-layer settings projection', () => {
  test('current metadata separates raw defaults and schema-resolved groups without writing missing keys', () => {
    const raw = { timeout: 0 }
    const defaults = { timeout: 30, flags: { verbose: false } }
    const resolved = { timeout: 0, flags: { verbose: false } }
    const state = bundle(raw, { timeout: 80 }, { timeout: 120, flags: { verbose: true } })
    state.global = { value: raw, has_override: true, default_value: defaults, layer_value: resolved }
    expect(runtimeSettingLayer(state, 'global', undefined)).toMatchObject({
      hasOverride: true,
      inheritedKnown: true,
      valueKnown: true,
      inherited: defaults,
      value: resolved,
      effectiveDiffers: true,
    })
    expect(state.global.value).toEqual({ timeout: 0 })
    expect(state.global.inherited_value).toBeUndefined()
  })

  test('resolved atomic model groups do not inherit missing preferences from the parent object', () => {
    const parent = { provider: 'p', model: 'parent', thinking_mode: 'high', parallel_tool_calls: false }
    const local = { provider: 'p', model: 'local' }
    const state = bundle(parent, local, { provider: 'environment', model: 'runtime' })
    state.workspace = { value: local, has_override: true, inherited_value: parent, layer_value: local }
    expect(runtimeSettingLayer(state, 'workspace', undefined).value).toEqual(local)
    expect(runtimeSettingLayer(state, 'workspace', undefined).inherited).toEqual(parent)
  })

  test('explicit null defaults and resolved values suppress legacy aliases and later-layer values', () => {
    const state = bundle(undefined, 'workspace', 'runtime')
    state.global = { default_value: null, layer_value: null, inherited_value: 'legacy-default' }
    expect(runtimeSettingBaseValue(state.global, 'global')).toBeNull()
    expect(runtimeSettingLayer(state, 'global', 'ui-default')).toMatchObject({
      inheritedKnown: true,
      valueKnown: true,
      inherited: null,
      value: null,
    })
    delete state.global.default_value
    expect(runtimeSettingBaseValue(state.global, 'global')).toBeUndefined()
    expect(runtimeSettingLayer(state, 'global', undefined).inheritedKnown).toBe(false)
    state.workspace = { inherited_value: null, default_value: 'builtin', layer_value: null }
    expect(runtimeSettingLayer(state, 'workspace', undefined).value).toBeNull()
  })

  test.each([[false], [0], [''], [[]]])(
    'new metadata preserves explicitly saved %p even when equal to the program default',
    (value) => {
      const state = bundle()
      state.global = { value, has_override: true, default_value: value, layer_value: value }
      expect(runtimeSettingLayer(state, 'global', 'ui-default')).toMatchObject({
        hasOverride: true,
        valueKnown: true,
        value,
        inherited: value,
      })
    },
  )

  test('authoritative schema presence preserves explicit null and distinguishes absence', () => {
    const state = bundle(null, undefined, 'parent')
    state.global.has_override = true
    expect(runtimeSettingLayer(state, 'global', '')).toMatchObject({ hasOverride: true, value: null })
    state.global.has_override = false
    expect(runtimeSettingLayer(state, 'global', '').hasOverride).toBe(false)
  })

  test('global editing ignores workspace and runtime overrides', () => {
    const state = runtimeSettingLayer(bundle(false, true, true), 'global', true)
    expect(state).toMatchObject({
      hasOverride: true,
      value: false,
      inherited: true,
      effective: true,
      effectiveDiffers: true,
    })
  })

  test('workspace editing uses authoritative parent metadata without importing the effective value', () => {
    const state = runtimeSettingLayer(bundle(undefined, undefined, false, true), 'workspace', false)
    expect(state).toMatchObject({ hasOverride: false, value: true, inherited: true, effectiveDiffers: true })
  })

  test.each([false, 0, ''])('explicit falsy value %p remains a local override', (value) => {
    expect(hasRuntimeLayerValue({ config_found: true, value })).toBe(true)
    expect(runtimeSettingLayer(bundle(value, undefined, value), 'global', true).hasOverride).toBe(true)
  })

  test('legacy scalar fallback remains an unverified edit default rather than a known parent', () => {
    const state = bundle(false, undefined, true)
    delete state.workspace.inherited_value
    expect(runtimeSettingLayer(state, 'workspace', true)).toMatchObject({
      value: false,
      inherited: false,
      inheritedKnown: false,
    })
    delete state.global.inherited_value
    state.global.value = undefined
    expect(runtimeSettingLayer(state, 'global', '').inheritedKnown).toBe(false)
  })
})

const file = readFileSync(new URL('../src/components/settings/ServerSettingField.vue', import.meta.url), 'utf8')
const compiled = transformSync(compileScript(parse(file).descriptor, { id: 'layered-server-field' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
const disposers: Array<() => void> = []
afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose()
  for (const scope of scopes.splice(0)) scope.stop()
})

async function editor(initial = bundle(), overrides: Record<string, unknown> = {}) {
  const props = vue.reactive({
    path: 'example.enabled',
    label: 'Example',
    kind: 'boolean',
    options: [],
    defaultValue: true,
    includeEmpty: false,
    emptyLabel: '',
    targetLayer: 'global',
    reload: true,
    disabled: false,
    ...overrides,
  })
  const state = structuredClone(initial)
  const writes: Array<{ action: string; path: string; value?: unknown; layer: string }> = []
  const events: Array<unknown[]> = []
  const mounted: Array<() => unknown> = []
  const confirmations: any[] = []
  const changeListeners = new Set<() => void>()
  let confirmResult: boolean | Promise<boolean> = true
  let failure = ''
  let writeWait: Promise<void> | undefined
  const mods: Record<string, unknown> = {
    vue: {
      ...vue,
      onMounted: (fn: () => unknown) => mounted.push(fn),
      onBeforeUnmount: (fn: () => void) => disposers.push(fn),
    },
    '@remixicon/vue': {},
    '@/composables/usePaneVisibility': { usePaneVisibility: () => vue.ref(true) },
    '@/i18n/settingsText': { settingsText: (key: string) => key },
    '@/lib/runtimeSettingLayer': { runtimeSettingLayer },
    '@/lib/appConfirm': {
      async confirmAction(options: unknown) {
        confirmations.push(options)
        return await confirmResult
      },
    },
    '@/lib/runtimeSettings': {
      subscribeRuntimeSettingsChanges(listener: () => void) {
        changeListeners.add(listener)
        return () => changeListeners.delete(listener)
      },
      async readRuntimeSettingSources() {
        return structuredClone(state)
      },
      displayJsonValue: (value: unknown) => (typeof value === 'string' ? value : JSON.stringify(value)),
      settingValue: (value: { value: unknown }, fallback: unknown) => value?.value ?? fallback,
      async setRuntimeSetting(path: string, value: unknown, _opts: unknown, layer: 'global' | 'workspace') {
        await writeWait
        if (failure) throw new Error(failure)
        writes.push({ action: 'set', path, value, layer })
        state[layer].value = value as any
        state[layer].has_override = true
        if (state[layer].layer_value !== undefined) state[layer].layer_value = value
      },
      async deleteRuntimeSetting(path: string, _opts: unknown, layer: 'global' | 'workspace') {
        await writeWait
        if (failure) throw new Error(failure)
        writes.push({ action: 'delete', path, layer })
        state[layer].value = undefined
        state[layer].has_override = false
        delete state[layer].override_path
        if (state[layer].layer_value !== undefined)
          state[layer].layer_value = runtimeSettingBaseValue(state[layer], layer)
      },
    },
  }
  const module = { exports: {} as { default: { setup: (props: unknown, ctx: unknown) => any } } }
  new Function('require', 'module', 'exports', compiled)(
    (id: string) => {
      if (id.endsWith('.vue')) return {}
      if (!(id in mods)) throw new Error('Unexpected dependency: ' + id)
      return mods[id]
    },
    module,
    module.exports,
  )
  const scope = vue.effectScope()
  scopes.push(scope)
  const panel = scope.run(() =>
    module.exports.default.setup(props, { emit: (...args: unknown[]) => events.push(args), expose() {} }),
  )
  for (const fn of mounted) await fn()
  return {
    panel,
    props,
    state,
    writes,
    events,
    confirmations,
    confirm(value: boolean | Promise<boolean>) {
      confirmResult = value
    },
    async notifyChange() {
      for (const listener of changeListeners) listener()
      await vue.nextTick()
      await Promise.resolve()
      await vue.nextTick()
    },
    fail: (message: string) => {
      failure = message
    },
    waitForWrite: (promise: Promise<void>) => {
      writeWait = promise
    },
  }
}

describe('layer source changes use the actual server field handlers', () => {
  test.each([
    ['boolean', true, false],
    ['number', 12, 0],
    ['select', 'global-theme', 'local-theme'],
    ['text', 'global-text', ''],
  ] as const)(
    'inherited %s fields reject direct editing until this layer is explicitly selected',
    async (kind, parent, next) => {
      const { panel, writes } = await editor(bundle(parent, undefined, 'later-runtime'), {
        targetLayer: 'workspace',
        kind,
        defaultValue: '',
      })
      const change = () => {
        if (kind === 'boolean') panel.onBooleanCheckbox(next)
        else if (kind === 'number') panel.onNumberInput({ target: { value: String(next) } })
        else if (kind === 'select') panel.onSelectValue(next)
        else panel.onTextInput({ target: { value: next } })
      }
      expect(panel.inheritedSource.value).toBe('Global layer')
      expect(panel.localValue.value).toBe(parent)
      expect(panel.valueDisabled.value).toBe(true)
      change()
      await panel.save()
      expect(writes).toEqual([])
      expect(panel.localValue.value).toBe(parent)
      await panel.chooseSource('local')
      expect(panel.valueDisabled.value).toBe(false)
      change()
      await panel.save()
      expect(writes).toEqual([{ action: 'set', path: 'example.enabled', value: next, layer: 'workspace' }])
      await panel.chooseSource('inherit')
      await panel.save()
      expect(panel.valueDisabled.value).toBe(true)
      expect(panel.localValue.value).toBe(parent)
    },
  )

  test.each([undefined, null])(
    'an empty parent %p enters an unwritten local draft before an explicit false is saved',
    async (parent) => {
      const initial = bundle(undefined, undefined, true)
      initial.workspace.inherited_value = parent
      initial.workspace.layer_value = true
      const { panel, writes } = await editor(initial, { targetLayer: 'workspace' })
      expect(panel.localValue.value).toBe('')
      expect(panel.valueDisabled.value).toBe(true)
      panel.onBooleanCheckbox(false)
      await panel.chooseSource('local')
      expect(panel.sourceMode.value).toBe('local')
      expect(panel.localValueReady.value).toBe(false)
      expect(panel.valueDisabled.value).toBe(false)
      await panel.save()
      expect(writes).toEqual([])
      panel.onBooleanCheckbox(false)
      await panel.save()
      expect(writes[0]).toMatchObject({ action: 'set', value: false, layer: 'workspace' })
    },
  )

  test('switching a pending reset back to local retains the original unsaved value', async () => {
    const { panel, writes } = await editor(bundle(1, 3, 3), {
      targetLayer: 'workspace',
      kind: 'number',
      defaultValue: 1,
    })
    panel.onNumberInput({ target: { value: '7' } })
    await panel.chooseSource('inherit')
    expect(panel.localValue.value).toBe(1)
    await panel.chooseSource('local')
    expect(panel.localValue.value).toBe(7)
    await panel.save()
    expect(writes).toEqual([{ action: 'set', path: 'example.enabled', value: 7, layer: 'workspace' }])
  })

  test.each([
    ['workspace', 'providers.default_selection', 'thinking_mode', 'high'],
    ['global', 'permission.tools.rules."shell.*"', 'routine', 'ask'],
  ] as const)(
    'editable atomic %s fields write their leaf and confirm resetting the group',
    async (layer, group, child, value) => {
      const initial = bundle()
      Object.assign(initial[layer], {
        value: undefined,
        has_override: true,
        override_path: group,
        requires_group_reset: false,
        default_value: null,
        inherited_value: layer === 'workspace' ? 'low' : undefined,
        layer_value: null,
      })
      const { panel, writes, confirmations, confirm } = await editor(initial, {
        targetLayer: layer,
        path: `${group}.${child}`,
        kind: 'select',
        includeEmpty: true,
        defaultValue: '',
      })
      expect(panel.valueDisabled.value).toBe(false)
      expect(panel.sourceMode.value).toBe('local')
      panel.onSelectValue(value)
      await panel.save()
      expect(writes[0]).toEqual({ action: 'set', path: `${group}.${child}`, value, layer })
      expect(confirmations).toEqual([])
      confirm(false)
      if (layer === 'workspace') await panel.onSelectValue('')
      else await panel.chooseSource('inherit')
      await panel.save()
      expect(writes).toHaveLength(1)
      expect(panel.sourceMode.value).toBe('local')
      confirm(true)
      if (layer === 'workspace') await panel.onSelectValue('')
      else await panel.chooseSource('inherit')
      await panel.save()
      expect(writes[1]).toEqual({ action: 'delete', path: group, layer })
    },
  )

  test('a current-protocol global field shows its resolved value and deletes only its raw override on reset', async () => {
    const initial = bundle(false, false, false)
    initial.global = { value: false, has_override: true, default_value: true, layer_value: false }
    const { panel, writes, state } = await editor(initial)
    expect(panel.localValue.value).toBe(false)
    expect(panel.inheritedLabel.value).toBe('Enabled')
    expect(panel.sourceMode.value).toBe('local')
    await panel.chooseSource('inherit')
    await panel.save()
    expect(writes).toEqual([{ action: 'delete', path: 'example.enabled', layer: 'global' }])
    expect(panel.localValue.value).toBe(true)
    expect(panel.layerState.value.effective).toBe(false)
    expect(state.workspace.value).toBe(false)
    panel.onBooleanCheckbox(true)
    await panel.save()
    expect(writes.at(-1)).toMatchObject({ action: 'set', value: true, layer: 'global' })
    expect(panel.sourceMode.value).toBe('local')
  })

  test('ancestor clearing locks leaf edits and only restores the confirmed group boundary', async () => {
    const initial = bundle(null)
    Object.assign(initial.global, { has_override: true, override_path: 'ui.tui', inherited_value: 'dark' })
    const { panel, writes, confirm, confirmations } = await editor(initial, {
      path: 'ui.tui.theme',
      kind: 'select',
      defaultValue: '',
      includeEmpty: true,
    })
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.groupOverridePath.value).toBe('ui.tui')
    expect(panel.valueDisabled.value).toBe(true)
    panel.onSelectValue('light')
    expect(panel.dirty.value).toBe(false)
    confirm(false)
    await panel.chooseSource('inherit')
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.sourceMode.value).toBe('local')
    confirm(true)
    await panel.chooseSource('inherit')
    await panel.save()
    expect(confirmations[1]).toMatchObject({
      title: 'Restore defaults for {path}?',
      description: 'This also restores the default values of the other settings in this group.',
      confirmText: 'Reset to defaults',
      cancelText: 'Cancel',
    })
    expect(writes).toEqual([{ action: 'delete', path: 'ui.tui', layer: 'global' }])
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.localValue.value).toBe('dark')
  })

  test('a group restore confirmation is discarded after its field target changes', async () => {
    const initial = bundle(null)
    Object.assign(initial.global, { has_override: true, override_path: 'ui.tui', inherited_value: 'dark' })
    const { panel, props, writes, confirm } = await editor(initial, { path: 'ui.tui.theme', kind: 'select' })
    let resolve!: (value: boolean) => void
    confirm(
      new Promise<boolean>((done) => {
        resolve = done
      }),
    )
    const choosing = panel.chooseSource('inherit')
    props.path = 'ui.tui.graphics'
    await vue.nextTick()
    resolve(true)
    await choosing
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.dirty.value).toBe(false)
  })

  test('workspace group restore retains inherited terminology and deletes only its boundary', async () => {
    const initial = bundle(undefined, null, 'dark')
    Object.assign(initial.workspace, { has_override: true, override_path: 'ui.tui', inherited_value: 'dark' })
    const { panel, writes, confirmations } = await editor(initial, {
      path: 'ui.tui.theme',
      kind: 'select',
      defaultValue: '',
      includeEmpty: true,
      targetLayer: 'workspace',
    })
    await panel.chooseSource('inherit')
    await panel.save()
    expect(confirmations[0]).toMatchObject({
      title: 'Restore inheritance for {path}?',
      confirmText: 'Use inherited value',
    })
    expect(writes).toEqual([{ action: 'delete', path: 'ui.tui', layer: 'workspace' }])
  })

  test.each([
    ['global', 'Default value'],
    ['workspace', 'Inherited value'],
  ])('%s source details name the real baseline', async (targetLayer, expected) => {
    const { panel } = await editor(bundle(), { targetLayer })
    expect(panel.sourceValueHeading.value).toBe(expected)
  })

  test('settings change notifications refresh clean fields while preserving unsaved drafts', async () => {
    const { panel, state, notifyChange } = await editor(bundle(true))
    state.global.value = false
    await notifyChange()
    expect(panel.localValue.value).toBe(false)
    panel.onBooleanCheckbox(true)
    await notifyChange()
    expect(panel.localValue.value).toBe(true)
    expect(panel.dirty.value).toBe(true)
  })

  test('loading a global field displays its raw false override despite effective true', async () => {
    const { panel, writes } = await editor(bundle(false, true, true))
    expect(panel.localValue.value).toBe(false)
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.layerState.value.effectiveDiffers).toBe(true)
    expect(writes).toHaveLength(0)
  })

  test.each(['workspace'])('pinning an inherited equal value in %s saves a real override', async (targetLayer) => {
    const { panel, writes } = await editor(bundle(), { targetLayer })
    expect(panel.sourceMode.value).toBe('inherit')
    panel.chooseSource('local')
    expect(panel.localValue.value).toBe(true)
    expect(panel.dirty.value).toBe(true)
    await panel.save()
    expect(writes).toEqual([{ action: 'set', path: 'example.enabled', value: true, layer: targetLayer }])
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.dirty.value).toBe(false)
  })

  test('global settings fill missing values and only write after direct edits or explicit reset', async () => {
    const { panel, writes, state } = await editor(bundle(undefined, false, false))
    expect(panel.localValue.value).toBe(true)
    expect(panel.dirty.value).toBe(false)
    await panel.chooseSource('local')
    await panel.save()
    expect(writes).toEqual([])
    panel.onBooleanCheckbox(false)
    await panel.save()
    expect(writes[0]).toEqual({ action: 'set', path: 'example.enabled', value: false, layer: 'global' })
    await panel.chooseSource('inherit')
    expect(panel.localValue.value).toBe(true)
    await panel.save()
    expect(writes[1]).toEqual({ action: 'delete', path: 'example.enabled', layer: 'global' })
    expect(state.workspace.value).toBe(false)
    expect(panel.dirty.value).toBe(false)
  })

  test('a missing global plugin theme displays the built-in palette without a reset option in its value menu', async () => {
    const initial = bundle(undefined, 'workspace-theme', 'workspace-theme')
    initial.global.inherited_value = null
    const { panel, writes, props } = await editor(initial, {
      path: 'ui.tui.theme',
      kind: 'select',
      includeEmpty: true,
      defaultValue: '',
      emptyLabel: 'Built-in palette',
    })
    expect(panel.localValue.value).toBe('')
    expect(panel.emptyValueDisplayLabel.value).toBe('Built-in palette')
    expect(panel.inheritedLabel.value).toBe('Built-in palette')
    expect(panel.allowsEmptyChoice.value).toBe(false)
    panel.onSelectValue('')
    await panel.save()
    expect(writes).toEqual([])
    panel.onSelectValue('plugin-theme')
    await panel.save()
    expect(writes[0]).toMatchObject({ action: 'set', value: 'plugin-theme', layer: 'global' })
    await panel.chooseSource('inherit')
    await panel.save()
    expect(writes[1]).toMatchObject({ action: 'delete', path: 'ui.tui.theme', layer: 'global' })
    expect(panel.emptyValueDisplayLabel.value).toBe('Built-in palette')
    props.targetLayer = 'workspace'
    await vue.nextTick()
    expect(panel.allowsEmptyChoice.value).toBe(true)
  })

  test('restoring inheritance deletes only the selected layer and shows its parent', async () => {
    const { panel, writes, state } = await editor(bundle(false, true, true), { targetLayer: 'workspace' })
    panel.chooseSource('inherit')
    expect(panel.localValue.value).toBe(false)
    await panel.save()
    expect(writes).toEqual([{ action: 'delete', path: 'example.enabled', layer: 'workspace' }])
    expect(state.global.value).toBe(false)
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.dirty.value).toBe(false)
  })

  test('editing a value directly changes its source to local, including zero', async () => {
    const initial = bundle(undefined, undefined, 50)
    initial.global.inherited_value = 50
    const { panel, writes } = await editor(initial, { kind: 'number', defaultValue: 50 })
    panel.onNumberInput({ target: { value: '0' } })
    await panel.save()
    expect(writes[0]).toMatchObject({ action: 'set', value: 0 })
    expect(panel.sourceMode.value).toBe('local')
  })

  test('inherit to local and back before autosave is a no-op', async () => {
    const { panel, writes } = await editor(bundle(), { targetLayer: 'workspace' })
    panel.chooseSource('local')
    panel.chooseSource('inherit')
    await panel.save()
    expect(panel.dirty.value).toBe(false)
    expect(writes).toHaveLength(0)
  })

  test('an unavailable parent cannot be pinned using an unrelated effective value', async () => {
    const initial = bundle(undefined, undefined, 'from-environment')
    delete initial.global.inherited_value
    const { panel, writes } = await editor(initial, { kind: 'text', defaultValue: '' })
    expect(panel.localDisabled.value).toBe(false)
    panel.chooseSource('local')
    await panel.save()
    expect(writes).toHaveLength(0)
    expect(panel.sourceMode.value).toBe('inherit')
  })

  test.each([true, false, 0])(
    'a UI default of %p cannot authorize pinning an unresolved parent',
    async (defaultValue) => {
      const initial = bundle(undefined, undefined, 'runtime')
      delete initial.global.inherited_value
      delete initial.workspace.inherited_value
      const { panel, writes } = await editor(initial, {
        kind: typeof defaultValue === 'number' ? 'number' : 'boolean',
        defaultValue,
      })
      expect(panel.inheritedLabel.value).toBe('Unavailable')
      expect(panel.localDisabled.value).toBe(false)
      expect(panel.localValue.value).toBe('')
      expect(panel.emptyValueDisplayLabel.value).toBe('Unavailable')
      panel.flushAutoSave()
      panel.chooseSource('local')
      await panel.save()
      expect(writes).toHaveLength(0)
    },
  )

  test('a known null parent cannot be fixed as a guessed non-null UI default', async () => {
    const initial = bundle(undefined, undefined, false)
    initial.global.inherited_value = null
    const { panel, writes } = await editor(initial, { defaultValue: true })
    expect(panel.localDisabled.value).toBe(false)
    expect(panel.inheritedLabel.value).toBe('No value')
    panel.chooseSource('local')
    await panel.save()
    expect(writes).toHaveLength(0)
  })

  test('an opaque explicit null is shown as local and can be restored to actual inheritance', async () => {
    const initial = bundle(null, undefined, null)
    initial.global.has_override = true
    initial.global.inherited_value = 'theme-parent'
    const { panel, writes } = await editor(initial, {
      path: 'ui.tui.theme',
      kind: 'select',
      includeEmpty: true,
      defaultValue: '',
      emptyLabel: 'Inherit',
    })
    expect(panel.sourceMode.value).toBe('local')
    expect(panel.emptyOptionLabel.value).toBe('No value')
    expect(panel.localValue.value).toBe('')
    panel.chooseSource('inherit')
    await panel.save()
    expect(writes).toEqual([{ action: 'delete', path: 'ui.tui.theme', layer: 'global' }])
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.localValue.value).toBe('theme-parent')
  })

  test('a failed reset retains a retryable inheritance draft', async () => {
    const { panel, writes, fail } = await editor(bundle(false))
    fail('write failed')
    panel.chooseSource('inherit')
    await panel.save()
    expect(panel.error.value).toBe('write failed')
    expect(panel.dirty.value).toBe(true)
    fail('')
    await panel.save()
    expect(writes).toEqual([{ action: 'delete', path: 'example.enabled', layer: 'global' }])
    expect(panel.dirty.value).toBe(false)
  })

  test('an in-flight write stays bound to its original field and layer', async () => {
    const { panel, props, writes, waitForWrite } = await editor()
    let finish!: () => void
    waitForWrite(
      new Promise<void>((resolve) => {
        finish = resolve
      }),
    )
    panel.onBooleanCheckbox(true)
    const pending = panel.save()
    props.path = 'other.enabled'
    props.targetLayer = 'workspace'
    await vue.nextTick()
    finish()
    await pending
    expect(writes[0]).toEqual({ action: 'set', path: 'example.enabled', value: true, layer: 'global' })
    expect(panel.sourceMode.value).toBe('inherit')
    expect(panel.dirty.value).toBe(false)
  })
})
