import { describe, expect, test } from 'bun:test'

import {
  buildDesktopConfigFacts,
  buildDesktopRuntimeFacts,
  buildDesktopStatusFacts,
  buildDesktopUpdateFacts,
} from './runtimeDesktopModel'

describe('runtimeDesktopModel', () => {
  test('buildDesktopConfigFacts summarizes config fields', () => {
    const facts = buildDesktopConfigFacts({
      autostart_on_boot: true,
      backend: {
        host: '127.0.0.1',
        port: 3210,
        cors_origins: [],
        cors_allow_all: false,
        agena_config_path: '/workspace/.agena/config.toml',
        agena_mode: 'default',
        workspace_root: '/workspace',
        database_path: '/workspace/agena.db',
        database_url: 'sqlite:///workspace/agena.db',
        backend_log_level: 'info',
        ui_cookie_samesite: 'lax',
        ui_password: null,
      },
    })

    expect(facts.find((fact) => fact.label === 'Autostart')?.value).toBe('enabled')
    expect(facts.find((fact) => fact.label === 'Workspace Root')?.value).toBe('/workspace')
  })

  test('buildDesktopStatusFacts exposes last error info', () => {
    const facts = buildDesktopStatusFacts({
      running: false,
      url: null,
      last_error: 'failed to start',
      last_error_info: {
        code: 'spawn_failed',
        summary: 'Spawn failed',
        detail: null,
        hint: 'Check binary path',
        exitCode: null,
        signal: null,
      },
    })

    expect(facts.find((fact) => fact.label === 'Running')?.value).toBe('no')
    expect(facts.find((fact) => fact.label === 'Hint')?.value).toBe('Check binary path')
  })

  test('buildDesktopRuntimeFacts returns installer metadata', () => {
    const facts = buildDesktopRuntimeFacts({
      installerVersion: '1.2.3',
      installerTarget: 'linux-x64',
      installerChannel: 'main',
      installerType: 'appimage',
      installerManager: 'self',
    })

    expect(facts.find((fact) => fact.label === 'Installer Version')?.value).toBe('1.2.3')
  })

  test('buildDesktopUpdateFacts formats byte progress', () => {
    const facts = buildDesktopUpdateFacts({
      running: true,
      kind: 'service',
      phase: 'downloading',
      message: 'Downloading update',
      downloadedBytes: 128,
      totalBytes: 256,
      error: null,
    })

    expect(facts.find((fact) => fact.label === 'Progress')?.value).toBe('128 / 256')
  })

  test('desktop fact builders handle empty inputs and partial update progress', () => {
    expect(buildDesktopConfigFacts(null)).toEqual([])
    expect(buildDesktopStatusFacts(null)).toEqual([])
    expect(buildDesktopRuntimeFacts(null)).toEqual([])
    expect(buildDesktopUpdateFacts(null)).toEqual([])

    const updateFacts = buildDesktopUpdateFacts({
      running: false,
      kind: '',
      phase: '',
      message: '',
      downloadedBytes: 512,
      totalBytes: null,
      error: 'network timeout',
    })

    expect(updateFacts.find((fact) => fact.label === 'Progress')?.value).toBe('512')
    expect(updateFacts.find((fact) => fact.label === 'Error')?.value).toBe('network timeout')
  })
})
