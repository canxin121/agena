import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as settings from '../src/lib/sandboxSettings'
import type { RuntimeSettingsLayer, RuntimeSettingsReadBundle } from '../src/lib/runtimeSettings'

type Policy = { sandbox?: boolean; resource_guard?: boolean }
function sources(global: Policy = {}, workspace: Policy = {}, effective: Policy = {}): settings.SandboxSettingSources {
  const bundle = (key: settings.SandboxSettingKey): RuntimeSettingsReadBundle => {
    const value = (policy: Policy) => policy[key]
    return {
      global: { value: value(global), default_value: true },
      workspace: { value: value(workspace), inherited_value: value(global) ?? true },
      effective: { value: value(effective) },
      file: { value: value(global) },
    }
  }
  return { sandbox: bundle('sandbox'), resource_guard: bundle('resource_guard') }
}

describe('independent sandbox and resource preflight settings', () => {
  test('server defaults enable safeguards, while unavailable settings remain unknown', () => {
    expect(settings.sandboxLayerSettings(null, 'global')).toEqual({ sandbox: undefined, resource_guard: undefined })
    expect(settings.sandboxExecutionState(sources())).toEqual({
      sandbox: true,
      resourceGuard: false,
      resourceGuardStandby: true,
      environmentDisabled: false,
    })
    expect(settings.SANDBOX_SETTING_KEYS).toEqual(['sandbox', 'resource_guard'])
  })

  test('each setting inherits independently while global editing ignores workspace overrides', () => {
    const bundle = sources({ sandbox: false, resource_guard: false }, { resource_guard: true })
    expect(settings.sandboxLayerSettings(bundle, 'global')).toEqual({ sandbox: false, resource_guard: false })
    expect(settings.sandboxLayerSettings(bundle, 'workspace')).toEqual({ sandbox: false, resource_guard: true })
    expect(settings.sandboxLayerOverride(bundle, 'workspace', 'sandbox')).toBeUndefined()
    expect(settings.sandboxLayerOverride(bundle, 'workspace', 'resource_guard')).toBe(true)
    expect(settings.sandboxExecutionState(bundle)).toEqual({
      sandbox: false,
      resourceGuard: true,
      resourceGuardStandby: false,
      environmentDisabled: false,
    })
  })

  test('turning off preflight preserves its own preference across sandbox changes', () => {
    const bundle = sources({ sandbox: true, resource_guard: false })
    expect(settings.sandboxExecutionState(bundle).resourceGuard).toBe(false)
    bundle.sandbox.global.value = false
    bundle.sandbox.workspace.inherited_value = false
    expect(settings.sandboxExecutionState(bundle).resourceGuard).toBe(false)
    expect(settings.sandboxLayerSettings(bundle, 'global').resource_guard).toBe(false)
    bundle.resource_guard.global.value = true
    bundle.resource_guard.workspace.inherited_value = true
    expect(settings.sandboxExecutionState(bundle).resourceGuard).toBe(true)
  })

  test('an explicit environment disable changes activity without changing saved preferences', () => {
    const bundle = sources({ sandbox: true, resource_guard: true }, {}, { sandbox: false, resource_guard: true })
    expect(settings.sandboxLayerSettings(bundle, 'global').sandbox).toBe(true)
    expect(settings.sandboxExecutionState(bundle)).toEqual({
      sandbox: false,
      resourceGuard: true,
      resourceGuardStandby: false,
      environmentDisabled: true,
    })
  })

  test('a persisted disable is not mislabeled as an environment override', () => {
    const bundle = sources({ sandbox: true }, { sandbox: false }, { sandbox: false })
    expect(settings.sandboxExecutionState(bundle).environmentDisabled).toBe(false)
  })
})

const source = readFileSync(new URL('../src/components/settings/SandboxSettingsPanel.vue', import.meta.url), 'utf8')
const { descriptor } = parse(source)
const compiled = transformSync(compileScript(descriptor, { id: 'sandbox-settings-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

async function editor(
  bundle = sources(),
  layer: RuntimeSettingsLayer = 'global',
  workspace?: { requested: string; returned: number; revision: string },
) {
  const props = vue.reactive({ scope: layer, workspaceId: workspace?.requested, activeWorkspace: !workspace })
  const state = structuredClone(bundle)
  const reads: string[] = []
  const writes: Array<{ path: string; value?: boolean; layer: RuntimeSettingsLayer }> = []
  const targets: Array<{ workspace_id?: string; expected_revision?: string }> = []
  const targetReads: string[] = []
  let failure = ''
  const keyForPath = (path: string) => {
    const key = settings.SANDBOX_SETTING_KEYS.find((key) => key === path)
    if (!key) throw new Error('Unexpected settings path: ' + path)
    return key
  }
  const modules: Record<string, unknown> = {
    vue: { ...vue, watch() {}, onBeforeUnmount() {} },
    '@remixicon/vue': {},
    '@/composables/usePaneVisibility': { usePaneVisibility: () => vue.ref(true) },
    '@/i18n/settingsText': { settingsText: (key: string) => key },
    '@/lib/sandboxSettings': settings,
    '@/lib/runtimeSettings': {
      subscribeRuntimeSettingsChanges: () => () => {},
      async getRuntimeSettingLayer(layer: string, path: string, { workspaceId }: { workspaceId: string }) {
        if (layer !== 'workspace' || !workspace) throw new Error('Unexpected workspace read')
        targetReads.push(workspaceId)
        return { ...state[keyForPath(path)].workspace, workspace_id: workspace.returned, revision: workspace.revision }
      },
      async readRuntimeSettingSources(path: string) {
        reads.push(path)
        return structuredClone(state[keyForPath(path)])
      },
      async setRuntimeSetting(
        path: string,
        value: boolean,
        options: (typeof targets)[number],
        layer: RuntimeSettingsLayer,
      ) {
        if (failure) throw new Error(failure)
        targets.push(options)
        writes.push({ path, value, layer })
        state[keyForPath(path)][layer].value = value
        if (layer === 'global') state[keyForPath(path)].workspace.inherited_value = value
      },
      async deleteRuntimeSetting(path: string, options: (typeof targets)[number], layer: RuntimeSettingsLayer) {
        if (failure) throw new Error(failure)
        targets.push(options)
        writes.push({ path, layer })
        state[keyForPath(path)][layer].value = undefined
        if (layer === 'global')
          state[keyForPath(path)].workspace.inherited_value = state[keyForPath(path)].global.default_value
      },
    },
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  new Function('require', 'module', 'exports', compiled)(
    (id: string) => {
      if (id.endsWith('.vue')) return {}
      if (!(id in modules)) throw new Error('Unexpected component dependency: ' + id)
      return modules[id]
    },
    module,
    module.exports,
  )
  const scope = vue.effectScope()
  scopes.push(scope)
  const panel = scope.run(() => module.exports.default!.setup(props, { expose() {}, emit() {} }))!
  await panel.refresh()
  return {
    panel,
    props,
    reads,
    writes,
    targets,
    targetReads,
    rejectSaves(message: string) {
      failure = message
    },
  }
}

describe('execution safeguards editor persistence', () => {
  test('selected workspace safeguards write with that workspace’s revision', async () => {
    const { panel, targets, targetReads } = await editor(sources({}, { sandbox: true }), 'workspace', {
      requested: '12',
      returned: 12,
      revision: 'selected-workspace',
    })
    await panel.save('sandbox', false)
    expect(targetReads).toEqual(['12', '12', '12', '12'])
    expect(targets).toEqual([{ reload: true, workspace_id: '12', expected_revision: 'selected-workspace' }])
  })

  test('a mismatched workspace response disables safeguard mutations', async () => {
    const { panel, writes } = await editor(sources({}, { sandbox: true }), 'workspace', {
      requested: '12',
      returned: 6,
      revision: 'another-workspace',
    })
    await panel.save('sandbox', false)
    expect(panel.error.value).toBe('The server did not return the selected workspace.')
    expect(writes).toEqual([])
  })

  test('workspace safeguards show their global value but reject edits until a local source is selected', async () => {
    const { panel, writes } = await editor(sources({ sandbox: false }, {}, { sandbox: true }), 'workspace')
    expect(panel.configured.value.sandbox).toBe(false)
    expect(panel.valueDisabled('sandbox')).toBe(true)
    await panel.save('sandbox', true)
    expect(writes).toEqual([])
    await panel.chooseSource('sandbox', 'local')
    expect(writes[0]).toEqual({ path: 'sandbox', value: false, layer: 'workspace' })
    expect(panel.valueDisabled('sandbox')).toBe(false)
    await panel.save('sandbox', true)
    expect(writes[1]).toEqual({ path: 'sandbox', value: true, layer: 'workspace' })
    await panel.chooseSource('sandbox', 'inherit')
    expect(writes[2]).toEqual({ path: 'sandbox', layer: 'workspace' })
    expect(panel.valueDisabled('sandbox')).toBe(true)
    expect(panel.valueDisabled('resource_guard')).toBe(true)
  })

  test('a cancelled empty local safeguard draft does not create an override', async () => {
    const bundle = sources()
    delete bundle.sandbox.workspace.inherited_value
    const { panel, writes } = await editor(bundle, 'workspace')
    await panel.chooseSource('sandbox', 'local')
    expect(panel.sourceMode('sandbox')).toBe('local')
    expect(panel.configured.value.sandbox).toBeUndefined()
    expect(panel.valueDisabled('sandbox')).toBe(false)
    expect(writes).toEqual([])
    await panel.chooseSource('sandbox', 'inherit')
    expect(panel.valueDisabled('sandbox')).toBe(true)
    expect(writes).toEqual([])
  })

  test('effective safeguard values remain read-only', async () => {
    const { panel, props, writes } = await editor(sources({ sandbox: true }, {}, { sandbox: false }))
    ;(props as { scope: string }).scope = 'effective'
    expect(panel.configured.value.sandbox).toBe(false)
    expect(panel.sourceMode('sandbox')).toBe('effective')
    expect(panel.valueDisabled('sandbox')).toBe(true)
    await panel.save('sandbox', true)
    await panel.chooseSource('sandbox', 'local')
    expect(writes).toEqual([])
  })

  test('an explicit null resolved safeguard does not display the default or final-runtime value', async () => {
    const bundle = sources({}, {}, { sandbox: true })
    Object.assign(bundle.sandbox.global, { default_value: true, layer_value: null, inherited_value: false })
    const { panel, writes } = await editor(bundle)
    expect(panel.configured.value.sandbox).toBeUndefined()
    expect(panel.configuredLabel('sandbox')).toBe('No value')
    expect(panel.inheritedLabel('sandbox')).toBe('Enabled')
    expect(panel.execution.value.sandbox).toBe(true)
    expect(writes).toHaveLength(0)
  })

  test('a known null parent cannot be pinned as the hardcoded enabled fallback', async () => {
    const bundle = sources()
    bundle.sandbox.global.default_value = null
    const { panel, writes } = await editor(bundle)
    expect(panel.inheritedLabel('sandbox')).toBe('No value')
    expect(panel.configured.value.sandbox).toBeUndefined()
    await panel.chooseSource('sandbox', 'local')
    expect(writes).toHaveLength(0)
  })

  test('global safeguard values edit directly and restore defaults without pinning a source', async () => {
    const bundle = sources()
    bundle.sandbox.global.default_value = true
    const { panel, writes } = await editor(bundle)
    expect(panel.configured.value.sandbox).toBe(true)
    await panel.chooseSource('sandbox', 'local')
    expect(writes).toEqual([])
    await panel.save('sandbox', false)
    expect(writes[0]).toEqual({ path: 'sandbox', value: false, layer: 'global' })
    await panel.chooseSource('sandbox', 'inherit')
    expect(writes[1]).toEqual({ path: 'sandbox', value: undefined, layer: 'global' })
    expect(panel.configured.value.sandbox).toBe(true)
  })

  test('pinning a known parent writes the same false value as a local override', async () => {
    const bundle = sources({ sandbox: false })
    bundle.sandbox.workspace.inherited_value = false
    const { panel, writes } = await editor(bundle, 'workspace')
    expect(panel.sourceMode('sandbox')).toBe('inherit')
    await panel.chooseSource('sandbox', 'local')
    expect(writes).toEqual([{ path: 'sandbox', value: false, layer: 'workspace' }])
    expect(panel.sourceMode('sandbox')).toBe('local')
  })

  test.each(['global', 'workspace'] as const)(
    'an unavailable %s parent cannot be pinned from a fallback',
    async (layer) => {
      const unavailable = sources()
      delete unavailable.sandbox.global.default_value
      delete unavailable.sandbox.workspace.inherited_value
      const { panel, writes } = await editor(unavailable, layer)
      expect(settings.sandboxInheritedKnown(panel.sources.value, layer, 'sandbox')).toBe(false)
      expect(panel.configured.value.sandbox).toBeUndefined()
      await panel.chooseSource('sandbox', 'local')
      expect(writes).toHaveLength(0)
      await panel.save('sandbox', false)
      expect(writes).toEqual([{ path: 'sandbox', value: false, layer }])
    },
  )

  test('loads both canonical paths and shows two separate controls', async () => {
    const { panel, reads, writes } = await editor()
    expect(reads).toEqual(['sandbox', 'resource_guard'])
    expect(panel.fields.value.map((field: { key: string }) => field.key)).toEqual(['sandbox', 'resource_guard'])
    expect(panel.fields.value.map((field: { label: string }) => field.label)).toEqual([
      'Native sandbox',
      'Resource preflight',
    ])
    expect(panel.configured.value).toEqual({ sandbox: true, resource_guard: true })
    expect(panel.fields.value[1].active).toBe(false)
    expect(panel.fields.value[1].status).toContain('On standby')
    expect(writes).toHaveLength(0)
  })

  test.each(['global', 'workspace'] as const)(
    'saving in %s writes each canonical setting independently',
    async (layer) => {
      const { panel, writes } = await editor(
        sources({}, layer === 'workspace' ? { sandbox: true, resource_guard: true } : {}),
        layer,
      )
      await panel.save('resource_guard', false)
      expect(writes).toEqual([{ path: 'resource_guard', value: false, layer }])
      expect(panel.configured.value.sandbox).toBe(true)
      expect(panel.configured.value.resource_guard).toBe(false)
      expect(panel.fields.value[1].status).toBe('')
      await panel.save('sandbox', false)
      expect(writes[1]).toEqual({ path: 'sandbox', value: false, layer })
      expect(panel.execution.value.resourceGuard).toBe(false)
      await panel.save('resource_guard', true)
      expect(panel.execution.value.resourceGuard).toBe(true)
    },
  )

  test('reset deletes only the selected field and restores its parent value', async () => {
    const { panel, writes } = await editor(
      sources({ sandbox: false, resource_guard: false }, { resource_guard: true }),
      'workspace',
    )
    expect(panel.configured.value).toEqual({ sandbox: false, resource_guard: true })
    await panel.save('resource_guard', undefined)
    expect(writes).toEqual([{ path: 'resource_guard', layer: 'workspace' }])
    expect(panel.configured.value).toEqual({ sandbox: false, resource_guard: false })
    expect(settings.sandboxLayerOverride(panel.sources.value, 'workspace', 'resource_guard')).toBeUndefined()
  })

  test('resetting the global preference restores its built-in true default', async () => {
    const { panel, writes } = await editor(sources({ sandbox: false, resource_guard: false }))
    await panel.save('resource_guard', undefined)
    expect(writes).toEqual([{ path: 'resource_guard', layer: 'global' }])
    expect(panel.configured.value).toEqual({ sandbox: false, resource_guard: true })
    expect(panel.execution.value.resourceGuard).toBe(true)
  })

  test.each(['global', 'workspace'] as const)(
    'resetting sandbox in %s deletes the root flag and keeps resource guard',
    async (layer) => {
      const bundle =
        layer === 'global'
          ? sources({ sandbox: false, resource_guard: false })
          : sources({ sandbox: true, resource_guard: false }, { sandbox: false })
      const { panel, writes } = await editor(bundle, layer)
      expect(panel.configured.value).toEqual({ sandbox: false, resource_guard: false })
      await panel.save('sandbox', undefined)
      expect(writes).toEqual([{ path: 'sandbox', layer }])
      expect(panel.configured.value).toEqual({ sandbox: true, resource_guard: false })
      expect(settings.sandboxLayerOverride(panel.sources.value, layer, 'sandbox')).toBeUndefined()
      expect(panel.execution.value.resourceGuard).toBe(false)
    },
  )

  test('a rejected save restores the displayed preference without changing the other control', async () => {
    const { panel, writes, rejectSaves } = await editor()
    rejectSaves('settings rejected')
    await panel.save('resource_guard', false)
    expect(writes).toHaveLength(0)
    expect(panel.error.value).toBe('settings rejected')
    expect(panel.configured.value).toEqual({ sandbox: true, resource_guard: true })
    expect(panel.pendingEdit.value).toBeNull()
  })
})
