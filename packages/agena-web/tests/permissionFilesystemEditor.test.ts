import { afterEach, describe, expect, test } from 'bun:test'
import { nextTick } from 'vue'
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
  test.each(['global', 'workspace', 'session'] as const)(
    'empty %s permissions display the inherited defaults without writing overrides',
    async (scope) => {
      const { panel, writes } = await createPermissionEditor({}, undefined, { section: 'filesystem', scope })
      expect(panel.modeAt('workspace')).toBe('')
      expect(panel.modeAt('external')).toBe('')
      expect(panel.pathInheritanceLabel('workspace')).toBe('Inherit · Read and write')
      expect(panel.pathInheritanceLabel('external')).toBe('Inherit · Deny')
      expect(panel.networkMode('default')).toBe('allow')
      expect(panel.networkMode('inbound')).toBe('deny')
      expect(panel.networkMode('hostLoopback')).toBe('deny')
      expect(panel.rawJson.value).toBe('{}')
      expect(panel.dirty.value).toBe(false)
      expect(writes).toHaveLength(0)
    },
  )

  test('resetting explicit restrictions returns to the server-provided defaults', async () => {
    const { panel, writes } = await createPermissionEditor(
      { path: { workspace: 'readonly', external: 'deny' }, network: {} },
      undefined,
      { section: 'filesystem' },
    )
    expect(panel.modeAt('workspace')).toBe('readonly')
    expect(panel.modeAt('external')).toBe('deny')
    expect(panel.networkMode('default')).toBe('deny')
    await panel.clearSource()
    expect(panel.rawJson.value).toBe('{}')
    expect(panel.pathInheritanceLabel('workspace')).toBe('Inherit · Read and write')
    expect(panel.networkMode('default')).toBe('allow')
    await panel.save()
    expect(writes).toEqual([{ path: 'permission', value: undefined, scope: 'global' }])
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
  })

  test('inheritance labels follow a restrictive parent instead of a UI default', async () => {
    const { panel } = await createPermissionEditor(
      {},
      { path: { workspace: 'readonly', external: 'deny' }, network: { default: 'deny' } },
      { section: 'filesystem' },
    )
    expect(panel.pathInheritanceLabel('workspace')).toBe('Inherit · Read only')
    expect(panel.networkMode('default')).toBe('deny')
    expect(panel.config.value).toEqual({})
  })

  test.each(['global', 'workspace'] as const)(
    'saving and refreshing %s reads back that layer independently of session policy',
    async (source) => {
      const initial = { path: { workspace: 'readonly' as const, external: 'deny' as const } }
      const { panel, reads, setServerPolicy, switchScope } = await createPermissionEditor(initial, null, {
        section: 'filesystem',
        scope: source,
        useRuntimeTransport: true,
      })
      const otherSource = source === 'global' ? 'workspace' : 'global'
      const other = { path: { workspace: 'deny' as const, external: 'deny' as const } }
      setServerPolicy(other, otherSource)
      panel.setPathDefault('workspace', 'readwrite')
      panel.setPathDefault('external', 'readonly')
      await panel.save()
      const saved = { path: { workspace: 'readwrite', external: 'readonly' } }
      await panel.refresh()
      expect(panel.config.value).toEqual(saved)
      expect(panel.savedConfig.value).toEqual(saved)
      expect(panel.dirty.value).toBe(false)
      expect(reads.filter((url) => url === `/api/v1/settings/layers/${source}?path=permission`)).toHaveLength(2)

      const effective = {
        path: {
          workspace: 'readwrite' as const,
          external: 'readonly' as const,
          rules: { '<state>': 'readwrite' as const },
        },
      }
      setServerPolicy(effective, 'effective')
      await switchScope('effective')
      expect(panel.config.value).toEqual(effective)
      expect(reads).toContain('/api/v1/sessions/1/state')
      await switchScope(otherSource)
      expect(panel.config.value).toEqual(other)
      await switchScope(source)
      expect(panel.config.value).toEqual(saved)
    },
  )

  test('the persisted response is authoritative while runtime reload is deferred', async () => {
    const initial = { path: { workspace: 'readonly' as const, external: 'deny' as const } }
    const { panel, reads, setSaveResponse, setServerPolicy } = await createPermissionEditor(initial, null, {
      section: 'filesystem',
      useRuntimeTransport: true,
    })
    panel.setPathDefault('workspace', 'readwrite')
    panel.setPathDefault('external', 'readonly')
    const confirmed = {
      path: { workspace: 'readwrite', external: 'readonly', rules: { '<workspace>/output': 'readwrite' } },
    }
    setSaveResponse({ current: confirmed, reload: { deferred_task_id: 'reload-1' } })
    setServerPolicy(initial, 'effective')
    const readsBeforeSave = [...reads]
    await panel.save()
    expect(panel.config.value).toEqual(confirmed)
    expect(panel.savedConfig.value).toEqual(confirmed)
    expect(panel.dirty.value).toBe(false)
    expect(reads).toEqual(readsBeforeSave)
    await panel.refresh()
    expect(panel.config.value).toEqual(confirmed)
  })

  test('an older edit response without current is confirmed against the same raw layer', async () => {
    const { panel, reads, setSaveResponse } = await createPermissionEditor({}, null, {
      section: 'filesystem',
      scope: 'workspace',
      useRuntimeTransport: true,
    })
    panel.setPathDefault('workspace', 'readwrite')
    panel.setPathDefault('external', 'readonly')
    setSaveResponse({ current: undefined })
    const before = reads.length
    await panel.save()
    expect(reads.slice(before)).toEqual(['/api/v1/settings/layers/workspace?path=permission'])
    expect(panel.savedConfig.value).toEqual({ path: { workspace: 'readwrite', external: 'readonly' } })
    expect(panel.dirty.value).toBe(false)
  })

  test.each(['global', 'workspace'] as const)(
    'unrelated conversation and directory changes cannot reset a pending %s save',
    async (source) => {
      const initial = { path: { workspace: 'readonly' as const, external: 'deny' as const } }
      const { panel, props, reads, lastSessionId, directory, holdSaves } = await createPermissionEditor(initial, null, {
        section: 'filesystem',
        scope: source,
        useRuntimeTransport: true,
      })
      panel.setPathDefault('workspace', 'readwrite')
      panel.setPathDefault('external', 'readonly')
      const key = panel.contextKey.value
      const readsBeforeSave = [...reads]
      const release = holdSaves()
      const pending = panel.save()
      expect(panel.saving.value).toBe(true)
      lastSessionId.value = '2'
      props.sessionId = '3'
      directory.currentDirectory = '/workspace/other'
      await nextTick()
      await Promise.resolve()
      await nextTick()
      expect(panel.contextKey.value).toBe(key)
      expect(reads).toEqual(readsBeforeSave)
      release()
      await pending
      expect(panel.config.value).toEqual({ path: { workspace: 'readwrite', external: 'readonly' } })
      expect(panel.dirty.value).toBe(false)
      expect(panel.saveError.value).toBe('')
    },
  )

  test('an invalid persisted response keeps the draft available for retry', async () => {
    const initial = { path: { workspace: 'readonly' as const, external: 'deny' as const } }
    const { panel, setSaveResponse, messages } = await createPermissionEditor(initial, null, {
      section: 'filesystem',
      useRuntimeTransport: true,
    })
    panel.setPathDefault('workspace', 'readwrite')
    panel.setPathDefault('external', 'readonly')
    setSaveResponse({ current: 'invalid permission response' })
    await panel.save()
    expect(panel.config.value).toEqual({ path: { workspace: 'readwrite', external: 'readonly' } })
    expect(panel.savedConfig.value).toEqual(initial)
    expect(panel.dirty.value).toBe(true)
    expect(panel.saveError.value).toContain('Expected an object')
    expect(messages).toHaveLength(0)
    setSaveResponse({})
    await panel.save()
    expect(panel.dirty.value).toBe(false)
    expect(panel.saveError.value).toBe('')
  })

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
