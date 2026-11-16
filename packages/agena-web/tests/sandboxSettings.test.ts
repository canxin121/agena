import { describe, expect, test } from 'bun:test'
import { sandboxExecutionState, sandboxLayerOverride, sandboxLayerSettings } from '../src/lib/sandboxSettings'
import type { RuntimeSettingsReadBundle } from '../src/lib/runtimeSettings'

function sources(global = {}, workspace = {}, effective = {}): RuntimeSettingsReadBundle {
  return {
    global: { value: global },
    workspace: { value: workspace },
    effective: { value: effective },
    file: { value: global },
  }
}

describe('sandbox settings layers and execution state', () => {
  test('defaults enable MXC and preserve both inactive fallback preferences', () => {
    expect(sandboxLayerSettings(null, 'global')).toEqual({ enabled: true, resource_checks: true, command_hints: true })
    expect(sandboxExecutionState(sources())).toEqual({
      enabled: true,
      resourceChecks: false,
      commandHints: false,
      environmentDisabled: false,
    })
  })

  test('workspace inherits fields independently and global editing ignores workspace overrides', () => {
    const bundle = sources({ enabled: false, resource_checks: false }, { resource_checks: true, command_hints: false })
    expect(sandboxLayerSettings(bundle, 'global')).toEqual({
      enabled: false,
      resource_checks: false,
      command_hints: true,
    })
    expect(sandboxLayerSettings(bundle, 'workspace')).toEqual({
      enabled: false,
      resource_checks: true,
      command_hints: false,
    })
    expect(sandboxLayerOverride(bundle, 'workspace', 'enabled')).toBeUndefined()
    expect(sandboxLayerOverride(bundle, 'workspace', 'command_hints')).toBe(false)
    expect(sandboxExecutionState(bundle)).toEqual({
      enabled: false,
      resourceChecks: true,
      commandHints: false,
      environmentDisabled: false,
    })
  })

  test('command hints require resource checks without clearing their saved preference', () => {
    const bundle = sources({ enabled: false, resource_checks: false, command_hints: true })
    expect(sandboxExecutionState(bundle).commandHints).toBe(false)
    expect(sandboxLayerSettings(bundle, 'global').command_hints).toBe(true)
    bundle.global.value.resource_checks = true
    expect(sandboxExecutionState(bundle).commandHints).toBe(true)
  })

  test('an environment disable changes runtime status without changing editable values', () => {
    const bundle = sources({ enabled: true }, {}, { enabled: false, resource_checks: true, command_hints: true })
    expect(sandboxLayerSettings(bundle, 'global').enabled).toBe(true)
    expect(sandboxExecutionState(bundle)).toEqual({
      enabled: false,
      resourceChecks: true,
      commandHints: true,
      environmentDisabled: true,
    })
  })

  test('a persisted disable is not mislabeled as an environment override', () => {
    expect(
      sandboxExecutionState(sources({ enabled: true }, { enabled: false }, { enabled: false })).environmentDisabled,
    ).toBe(false)
  })
})
