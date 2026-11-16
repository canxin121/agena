import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import * as settings from '../src/lib/sandboxSettings'
import type { RuntimeSettingsLayer, RuntimeSettingsReadBundle } from '../src/lib/runtimeSettings'

type Policy = { sandbox?: { enabled?: boolean }; resource_guard?: boolean }
function sources(global: Policy = {}, workspace: Policy = {}, effective: Policy = {}): settings.SandboxSettingSources {
  const bundle = (key: settings.SandboxSettingKey): RuntimeSettingsReadBundle => {
    const value = (policy: Policy) => key === 'enabled' ? policy.sandbox?.enabled : policy.resource_guard
    return {
      global: { value: value(global) },
      workspace: { value: value(workspace) },
      effective: { value: value(effective) },
      file: { value: value(global) },
    }
  }
  return { enabled: bundle('enabled'), resource_guard: bundle('resource_guard') }
}

describe('independent sandbox and resource preflight settings', () => {
  test('both preferences default to true and resource preflight waits while MXC is enabled', () => {
    expect(settings.sandboxLayerSettings(null, 'global')).toEqual({ enabled: true, resource_guard: true })
    expect(settings.sandboxExecutionState(sources())).toEqual({
      enabled: true,
      resourceGuard: false,
      resourceGuardStandby: true,
      environmentDisabled: false,
    })
    expect(settings.SANDBOX_SETTING_PATHS).toEqual({ enabled: 'sandbox.enabled', resource_guard: 'resource_guard' })
  })

  test('each setting inherits independently while global editing ignores workspace overrides', () => {
    const bundle = sources(
      { sandbox: { enabled: false }, resource_guard: false },
      { resource_guard: true },
    )
    expect(settings.sandboxLayerSettings(bundle, 'global')).toEqual({ enabled: false, resource_guard: false })
    expect(settings.sandboxLayerSettings(bundle, 'workspace')).toEqual({ enabled: false, resource_guard: true })
    expect(settings.sandboxLayerOverride(bundle, 'workspace', 'enabled')).toBeUndefined()
    expect(settings.sandboxLayerOverride(bundle, 'workspace', 'resource_guard')).toBe(true)
    expect(settings.sandboxExecutionState(bundle)).toEqual({
      enabled: false,
      resourceGuard: true,
      resourceGuardStandby: false,
      environmentDisabled: false,
    })
  })

  test('turning off preflight preserves its own preference across sandbox changes', () => {
    const bundle = sources({ sandbox: { enabled: true }, resource_guard: false })
    expect(settings.sandboxExecutionState(bundle).resourceGuard).toBe(false)
    bundle.enabled.global.value = false
    expect(settings.sandboxExecutionState(bundle).resourceGuard).toBe(false)
    expect(settings.sandboxLayerSettings(bundle, 'global').resource_guard).toBe(false)
    bundle.resource_guard.global.value = true
    expect(settings.sandboxExecutionState(bundle).resourceGuard).toBe(true)
  })

  test('an explicit environment disable changes activity without changing saved preferences', () => {
    const bundle = sources(
      { sandbox: { enabled: true }, resource_guard: true },
      {},
      { sandbox: { enabled: false }, resource_guard: true },
    )
    expect(settings.sandboxLayerSettings(bundle, 'global').enabled).toBe(true)
    expect(settings.sandboxExecutionState(bundle)).toEqual({
      enabled: false,
      resourceGuard: true,
      resourceGuardStandby: false,
      environmentDisabled: true,
    })
  })

  test('a persisted disable is not mislabeled as an environment override', () => {
    const bundle = sources({ sandbox: { enabled: true } }, { sandbox: { enabled: false } }, { sandbox: { enabled: false } })
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

async function editor(bundle = sources(), layer: RuntimeSettingsLayer = 'global') {
  const props = vue.reactive({ scope: layer })
  const state = structuredClone(bundle)
  const reads: string[] = []
  const writes: Array<{ path: string; value?: boolean; layer: RuntimeSettingsLayer }> = []
  let failure = ''
  const keyForPath = (path: string) => {
    const key = settings.SANDBOX_SETTING_KEYS.find((key) => settings.SANDBOX_SETTING_PATHS[key] === path)
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
      async readRuntimeSettingSources(path: string) {
        reads.push(path)
        return structuredClone(state[keyForPath(path)])
      },
      async setRuntimeSetting(path: string, value: boolean, _options: unknown, layer: RuntimeSettingsLayer) {
        if (failure) throw new Error(failure)
        writes.push({ path, value, layer })
        state[keyForPath(path)][layer].value = value
      },
      async deleteRuntimeSetting(path: string, _options: unknown, layer: RuntimeSettingsLayer) {
        if (failure) throw new Error(failure)
        writes.push({ path, layer })
        state[keyForPath(path)][layer].value = undefined
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
  const panel = scope.run(() => module.exports.default!.setup(props, { expose() {} }))!
  await panel.refresh()
  return {
    panel, props, reads, writes,
    rejectSaves(message: string) { failure = message },
  }
}

describe('execution safeguards editor persistence', () => {
  test('loads both canonical paths and shows two separate controls', async () => {
    const { panel, reads, writes } = await editor()
    expect(reads).toEqual(['sandbox.enabled', 'resource_guard'])
    expect(panel.fields.value.map((field: { key: string }) => field.key)).toEqual(['enabled', 'resource_guard'])
    expect(panel.fields.value.map((field: { label: string }) => field.label)).toEqual(['Native sandbox', 'Resource preflight'])
    expect(panel.configured.value).toEqual({ enabled: true, resource_guard: true })
    expect(panel.fields.value[1].active).toBe(false)
    expect(panel.fields.value[1].status).toContain('On standby')
    expect(writes).toHaveLength(0)
  })

  test.each(['global', 'workspace'] as const)('saving in %s writes each canonical setting independently', async (layer) => {
    const { panel, writes } = await editor(sources(), layer)
    await panel.save('resource_guard', false)
    expect(writes).toEqual([{ path: 'resource_guard', value: false, layer }])
    expect(panel.configured.value.enabled).toBe(true)
    expect(panel.configured.value.resource_guard).toBe(false)
    expect(panel.fields.value[1].status).toBe('')
    await panel.save('enabled', false)
    expect(writes[1]).toEqual({ path: 'sandbox.enabled', value: false, layer })
    expect(panel.execution.value.resourceGuard).toBe(false)
    await panel.save('resource_guard', true)
    expect(panel.execution.value.resourceGuard).toBe(true)
  })

  test('reset deletes only the selected field and restores its parent value', async () => {
    const { panel, writes } = await editor(
      sources({ sandbox: { enabled: false }, resource_guard: false }, { resource_guard: true }),
      'workspace',
    )
    expect(panel.configured.value).toEqual({ enabled: false, resource_guard: true })
    await panel.save('resource_guard', undefined)
    expect(writes).toEqual([{ path: 'resource_guard', layer: 'workspace' }])
    expect(panel.configured.value).toEqual({ enabled: false, resource_guard: false })
    expect(settings.sandboxLayerOverride(panel.sources.value, 'workspace', 'resource_guard')).toBeUndefined()
  })

  test('resetting the global preference restores its built-in true default', async () => {
    const { panel, writes } = await editor(sources({ sandbox: { enabled: false }, resource_guard: false }))
    await panel.save('resource_guard', undefined)
    expect(writes).toEqual([{ path: 'resource_guard', layer: 'global' }])
    expect(panel.configured.value).toEqual({ enabled: false, resource_guard: true })
    expect(panel.execution.value.resourceGuard).toBe(true)
  })

  test('a rejected save restores the displayed preference without changing the other control', async () => {
    const { panel, writes, rejectSaves } = await editor()
    rejectSaves('settings rejected')
    await panel.save('resource_guard', false)
    expect(writes).toHaveLength(0)
    expect(panel.error.value).toBe('settings rejected')
    expect(panel.configured.value).toEqual({ enabled: true, resource_guard: true })
    expect(panel.pendingEdit.value).toBeNull()
  })
})
