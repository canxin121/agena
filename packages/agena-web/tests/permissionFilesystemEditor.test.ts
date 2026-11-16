import { afterEach, describe, expect, test } from 'bun:test'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

const approvalModel = {
  adapter: 'openai_responses',
  model: 'gpt-6-luna',
  provider: 'local',
  speed_mode: 'fast',
  thinking_mode: 'medium',
}

describe('filesystem permission editor persistence', () => {
  test('workspace readwrite and external readonly produce the reported settings request without changing approval model', async () => {
    const { panel, requests, writes } = await createPermissionEditor(
      { approval_model: approvalModel },
      { path: { workspace: 'deny', external: 'deny' }, network: { default: 'deny' } },
      { section: 'filesystem', useRuntimeTransport: true },
    )
    panel.setPathDefault('workspace', 'readwrite')
    panel.setPathDefault('external', 'readonly')
    expect(panel.modeAt('workspace')).toBe('readwrite')
    expect(panel.modeAt('external')).toBe('readonly')
    const permission = { path: { workspace: 'readwrite', external: 'readonly' }, approval_model: approvalModel }
    await panel.save()
    expect(requests).toEqual([
      {
        url: '/api/v1/settings/layers/global',
        method: 'PUT',
        body: { path: 'permission', value: permission, dry_run: false, validate: true, reload: true },
      },
    ])
    expect(writes).toHaveLength(1)
    expect(panel.config.value).toEqual(permission)
    expect(panel.savedConfig.value).toEqual(permission)
    expect(panel.dirty.value).toBe(false)
    expect(panel.saveError.value).toBe('')
  })

  test('saving path defaults retains unrelated network, tool, and approval settings', async () => {
    const original = {
      network: { default: 'deny' as const, allow: [{ to: '192.0.2.10', protocol: 'tcp' as const, ports: [443] }] },
      tools: { default: 'ask' as const, names: { 'agena.fs.read': 'allow' as const } },
      approval_model: approvalModel,
    }
    const { panel, writes } = await createPermissionEditor(original, null, {
      section: 'filesystem',
      useRuntimeTransport: true,
    })
    panel.setPathDefault('workspace', 'readwrite')
    panel.setPathDefault('external', 'readonly')
    await panel.save()
    expect(writes[0]?.value).toEqual({ ...original, path: { workspace: 'readwrite', external: 'readonly' } })
  })

  test('backend rejection keeps the full editable draft and error, and retry saves the same candidate', async () => {
    const original = { path: { workspace: 'deny' as const, external: 'deny' as const }, approval_model: approvalModel }
    const { panel, requests, writes, messages, rejectSaves } = await createPermissionEditor(original, null, {
      section: 'filesystem',
      useRuntimeTransport: true,
    })
    panel.setPathDefault('workspace', 'readwrite')
    panel.setPathDefault('external', 'readonly')
    const candidate = { path: { workspace: 'readwrite', external: 'readonly' }, approval_model: approvalModel }
    const detail = 'MXC filesystem policy rejected permission.path.external\nDetailed native validation reason.'
    rejectSaves(detail)
    await panel.save()
    expect(requests).toHaveLength(1)
    expect(writes).toHaveLength(0)
    expect(panel.saveError.value).toBe(detail)
    expect(panel.config.value).toEqual(candidate)
    expect(panel.savedConfig.value).toEqual(original)
    expect(panel.dirty.value).toBe(true)
    expect(messages).toHaveLength(0)
    rejectSaves('')
    await panel.save()
    expect(requests[1]).toEqual(requests[0])
    expect(writes).toHaveLength(1)
    expect(panel.savedConfig.value).toEqual(candidate)
    expect(panel.dirty.value).toBe(false)
  })

  test('restoring inherited defaults removes only those overrides and keeps sibling policy fields', async () => {
    const rules = { '<workspace>/secrets': 'deny' as const }
    const { panel, requests } = await createPermissionEditor(
      {
        path: { workspace: 'readwrite', external: 'readonly', rules },
        network: { default: 'deny' },
        approval_model: approvalModel,
      },
      { path: { workspace: 'readonly', external: 'deny' }, network: { default: 'allow' } },
      { section: 'filesystem', scope: 'workspace', useRuntimeTransport: true },
    )
    panel.setPathDefault('workspace', '')
    panel.setPathDefault('external', '')
    expect(panel.modeAt('workspace')).toBe('')
    expect(panel.modeAt('external')).toBe('')
    await panel.save()
    expect(requests[0]).toEqual({
      url: '/api/v1/settings/layers/workspace',
      method: 'PUT',
      body: {
        path: 'permission',
        value: { path: { rules }, network: { default: 'deny' }, approval_model: approvalModel },
        dry_run: false,
        validate: true,
        reload: true,
      },
    })
    expect(panel.dirty.value).toBe(false)
  })

  test('restoring inheritance for the last path defaults removes the selected permission override', async () => {
    const { panel, requests } = await createPermissionEditor(
      { path: { workspace: 'readwrite', external: 'readonly' } },
      { path: { workspace: 'readonly', external: 'deny' }, network: { default: 'deny' } },
      { section: 'filesystem', scope: 'workspace', useRuntimeTransport: true },
    )
    panel.setPathDefault('workspace', '')
    panel.setPathDefault('external', '')
    await panel.save()
    expect(requests[0]).toEqual({
      url: '/api/v1/settings/layers/workspace?path=permission&dry_run=false&validate=true&reload=true',
      method: 'DELETE',
      body: undefined,
    })
    expect(panel.savedConfig.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
  })

  test.each([
    {
      '<workspace>/tree': 'readonly',
      '<workspace>/tree/output': 'readwrite',
      '<workspace>/tree/output/reference': 'readonly',
    },
    {
      '<workspace>/tree': 'readwrite',
      '<workspace>/tree/reference': 'readonly',
      '<workspace>/tree/reference/output': 'readwrite',
    },
  ])('mixed directory modes reach authoritative backend validation: %p', async (rules) => {
    const { panel, requests } = await createPermissionEditor({}, null, {
      section: 'filesystem',
      useRuntimeTransport: true,
    })
    panel.rawJson.value = JSON.stringify({ path: { workspace: 'readwrite', external: 'readonly', rules } })
    await panel.save()
    expect(requests).toHaveLength(1)
    expect(panel.savedConfig.value.path.rules).toEqual(rules)
    expect(panel.dirty.value).toBe(false)
  })
})
