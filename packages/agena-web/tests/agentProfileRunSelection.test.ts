import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { transformSync } from 'esbuild'
import { deriveSendRunConfig } from '../src/pages/chat/modelSendDefaults'
import { buildRunRequestBody } from '../src/stores/chat/runOptions'
import { buildSessionSelectionRequest } from '../src/stores/chat/api'

const effective = {
  selectedProviderId: 'local',
  selectedAdapterId: 'openai',
  selectedModelId: 'profile-model',
  selectedThinkingMode: 'high',
  selectedSpeedMode: 'fast',
}

test('displayed profile and runtime defaults never become explicit run overrides', () => {
  for (const source of ['session', 'default', 'empty'] as const) {
    expect(
      deriveSendRunConfig({ ...effective, modelSource: source, thinkingModeSource: source, speedModeSource: source }),
    ).toEqual({})
  }
  expect(deriveSendRunConfig(effective)).toEqual({})
})

test('manual choices are sent independently of inherited profile fields', () => {
  expect(
    deriveSendRunConfig({
      ...effective,
      modelSource: 'manual',
      thinkingModeSource: 'session',
      speedModeSource: 'model',
    }),
  ).toEqual({
    providerID: 'local',
    adapterID: 'openai',
    modelID: 'profile-model',
  })
  expect(
    deriveSendRunConfig({
      ...effective,
      modelSource: 'session',
      thinkingModeSource: 'manual',
      speedModeSource: 'manual',
    }),
  ).toEqual({
    thinkingMode: 'high',
    speedMode: 'fast',
  })
})

test('mode reset requests preserve explicit empty strings only at the selection endpoint', () => {
  expect(buildSessionSelectionRequest({})).toEqual({})
  expect(buildSessionSelectionRequest({ thinking_mode: '' })).toEqual({ thinking_mode: '' })
  expect(buildSessionSelectionRequest({ speed_mode: '', verbosity: '' })).toEqual({ speed_mode: '', verbosity: '' })
  expect(buildRunRequestBody({ thinking_mode: '', speed_mode: '' })).toEqual({})
})

const runtimeSettingsSource = readFileSync(new URL('../src/lib/runtimeSettings.ts', import.meta.url), 'utf8')
const compiled = transformSync(runtimeSettingsSource, { loader: 'ts', format: 'cjs' }).code
function settingsTransport() {
  const invalidations: Array<[string, string]> = []
  const notifications: Array<[number, boolean]> = []
  const modules: Record<string, any> = {
    '@/lib/api': {
      async apiJson() {
        return { changed: true }
      },
    },
    './modelConfigurationApi': {
      notifyModelConfigurationChanged(scope: number, inventory: boolean) {
        notifications.push([scope, inventory])
      },
    },
    './resourceSync': {
      captureResourceObservation: () => ({ scope: 3 }),
      invalidateResourcePrefix(prefix: string, reason: string) {
        invalidations.push([prefix, reason])
      },
    },
    './runtimeSettingReads': { retireRuntimeSettingReads() {}, readBatchedRuntimeSettingSources() {} },
  }
  const module = { exports: {} as any }
  new Function('require', 'module', 'exports', compiled)(
    (id: string) => {
      if (!(id in modules)) throw new Error(`Unexpected dependency: ${id}`)
      return modules[id]
    },
    module,
    module.exports,
  )
  return { transport: module.exports, invalidations, notifications }
}

test('profile catalog and default changes retire live session projections after reload', async () => {
  const { transport, invalidations, notifications } = settingsTransport()
  await transport.setRuntimeSetting('profiles', { review: {} }, undefined, 'workspace')
  await transport.setRuntimeSetting('default_profile', 'review', undefined, 'workspace')
  expect(invalidations).toEqual([
    ['session:', 'mutation'],
    ['session:', 'mutation'],
  ])
  expect(notifications).toEqual([
    [3, false],
    [3, false],
  ])
})

test('dry-run and deferred profile writes leave live model state alone', async () => {
  const { transport, invalidations, notifications } = settingsTransport()
  await transport.setRuntimeSetting('profiles.review', {}, { dry_run: true }, 'global')
  await transport.setRuntimeSetting('profiles.review', {}, { reload: false }, 'global')
  expect(invalidations).toEqual([])
  expect(notifications).toEqual([])
})
