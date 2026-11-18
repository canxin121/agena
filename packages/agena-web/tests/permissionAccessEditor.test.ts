import { afterEach, describe, expect, test } from 'bun:test'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

describe('unified access permission editor', () => {
  test.each(['global', 'workspace', 'session'] as const)(
    'saves all three domains in one %s mutation',
    async (scope) => {
      const { panel, writes } = await createPermissionEditor(
        { tools: { names: { 'agena.fs.read': 'allow' } } },
        undefined,
        { scope, useRuntimeTransport: scope !== 'session' },
      )
      panel.setPathDefault('workspace', 'readonly')
      panel.setNetworkSource('custom')
      panel.setNetworkMode('default', 'deny')
      panel.setToolDefault('ask')
      panel.setToolNameMode('agena.shell.exec', 'deny')
      await panel.save()
      expect(writes).toHaveLength(1)
      expect(writes[0]).toEqual({
        path: 'permission',
        scope,
        value: {
          path: { workspace: 'readonly' },
          network: { default: 'deny', inbound: 'deny', hostLoopback: 'deny' },
          tools: { default: 'ask', names: { 'agena.fs.read': 'allow', 'agena.shell.exec': 'deny' } },
        },
      })
      expect(panel.dirty.value).toBe(false)
    },
  )

  test('one draft retains pending changes in every domain across a failed save and refresh', async () => {
    const editor = await createPermissionEditor()
    editor.panel.setPathDefault('external', 'deny')
    editor.panel.setNetworkSource('custom')
    editor.panel.setNetworkMode('default', 'deny')
    editor.panel.setToolDefault('auto')
    const draft = JSON.parse(JSON.stringify(editor.panel.config.value))
    editor.rejectSaves('fixture failure')
    await editor.panel.save()
    await editor.panel.refresh()
    expect(editor.writes).toHaveLength(0)
    expect(editor.panel.config.value).toEqual(draft)
    expect(editor.panel.dirty.value).toBe(true)
    editor.rejectSaves('')
    await editor.panel.save()
    expect(editor.writes[0]?.value).toEqual(draft)
  })

  test('reactive scope context changes keep drafts separate and restore all three domains', async () => {
    const editor = await createPermissionEditor()
    editor.panel.setPathDefault('workspace', 'readonly')
    editor.panel.setNetworkSource('custom')
    editor.panel.setNetworkMode('default', 'deny')
    editor.panel.setToolDefault('ask')
    const globalDraft = JSON.parse(JSON.stringify(editor.panel.config.value))
    await editor.switchScope('workspace')
    expect(editor.panel.config.value).toEqual({})
    editor.panel.setToolDefault('deny')
    await editor.switchScope('global')
    expect(editor.panel.config.value).toEqual(globalDraft)
    await editor.panel.save()
    expect(editor.writes[0]?.scope).toBe('global')
    await editor.switchScope('workspace')
    expect(editor.panel.config.value).toEqual({ tools: { default: 'deny' } })
  })

  test.each(['overview', 'sandbox'])('leaving for %s confirms the entire unsaved draft', async (view) => {
    const editor = await createPermissionEditor()
    editor.panel.setToolDefault('deny')
    editor.setConfirmation(false)
    expect(await editor.navigate(view)).toBe(false)
    expect(editor.panel.config.value.tools?.default).toBe('deny')
    expect(editor.confirmations).toHaveLength(1)
    editor.setConfirmation(true)
    expect(await editor.navigate(view)).toBe(true)
    expect(editor.panel.dirty.value).toBe(false)
  })

  test('route scope changes require confirmation and cancellation keeps every permission domain', async () => {
    const editor = await createPermissionEditor()
    editor.panel.setPathDefault('workspace', 'readonly')
    editor.panel.setNetworkSource('custom')
    editor.panel.setNetworkMode('default', 'deny')
    editor.panel.setToolDefault('ask')
    const draft = JSON.parse(JSON.stringify(editor.panel.config.value))
    editor.setConfirmation(false)
    expect(await editor.navigate('access', 'workspace')).toBe(false)
    expect(editor.panel.config.value).toEqual(draft)
    expect(editor.panel.dirty.value).toBe(true)
    editor.setConfirmation(true)
    expect(await editor.navigate('access', 'workspace')).toBe(true)
    expect(editor.panel.config.value).toEqual({})
    expect(editor.panel.dirty.value).toBe(false)
  })

  test('section navigation and old same-scope links keep the same unsaved draft', async () => {
    const editor = await createPermissionEditor()
    editor.panel.setPathDefault('external', 'deny')
    editor.setConfirmation(false)
    for (const view of ['access', 'filesystem', 'network', 'tools']) expect(await editor.navigate(view)).toBe(true)
    editor.panel.selectSection('network')
    expect(editor.route.hash).toBe('#permission-network')
    expect(editor.route.query.scope).toBe('global')
    expect(editor.panel.config.value.path?.external).toBe('deny')
    expect(editor.confirmations).toHaveLength(0)
    expect(editor.writes).toHaveLength(0)
  })

  test('effective access remains read only across every domain', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'effective' })
    editor.panel.setPathDefault('workspace', 'readwrite')
    editor.panel.setNetworkSource('custom')
    editor.panel.setToolDefault('allow')
    await editor.panel.save()
    expect(editor.panel.config.value).toEqual({})
    expect(editor.writes).toHaveLength(0)
  })

  test('session target changes confirm unsaved work and cancellation keeps the original target and draft', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'session', sessionId: '1' })
    editor.setSessionPolicy('2', { tools: { default: 'ask' } })
    editor.panel.setPathDefault('external', 'deny')
    editor.panel.setToolDefault('deny')
    const draft = JSON.parse(JSON.stringify(editor.panel.config.value))
    editor.setConfirmation(false)

    expect(await editor.navigateSession('2')).toBe(false)
    expect(editor.props.sessionId).toBe('1')
    expect(editor.route.query.targetSessionId).toBe('1')
    expect(editor.panel.activeSessionId.value).toBe(1)
    expect(editor.panel.config.value).toEqual(draft)
    expect(editor.confirmations).toHaveLength(1)

    editor.setConfirmation(true)
    expect(await editor.navigateSession('2')).toBe(true)
    await editor.panel.load()
    expect(editor.panel.activeSessionId.value).toBe(2)
    expect(editor.panel.config.value).toEqual({ tools: { default: 'ask' } })
    expect(editor.panel.dirty.value).toBe(false)
    editor.panel.setToolDefault('allow')
    await editor.panel.save()
    expect(editor.sessionWrites).toEqual([{ sessionId: '2', value: { tools: { default: 'allow' } } }])
    expect(editor.lastSessionId.value).toBe('1')
  })

  test('same-target links keep a session draft while a pending save blocks a different target', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'session', sessionId: '1' })
    editor.panel.setToolDefault('ask')
    editor.setConfirmation(false)
    expect(await editor.navigate('network')).toBe(true)
    expect(await editor.navigateSession('001')).toBe(true)
    expect(editor.confirmations).toHaveLength(0)

    const release = editor.holdSaves()
    const saving = editor.panel.save()
    expect(await editor.navigateSession('2')).toBe(false)
    expect(editor.panel.activeSessionId.value).toBe(1)
    release()
    await saving
    expect(editor.sessionWrites[0]?.sessionId).toBe('1')
    expect(editor.panel.dirty.value).toBe(false)
  })

  test('a late response from the previous session cannot replace the selected session policy', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'session', sessionId: '1' })
    editor.setSessionPolicy('1', { tools: { default: 'deny' } })
    editor.setSessionPolicy('2', { tools: { default: 'allow' } })
    const release = editor.holdSessionReads('1')
    const oldRead = editor.panel.refresh()
    expect(await editor.navigateSession('2')).toBe(true)
    await editor.panel.load()
    expect(editor.panel.config.value.tools?.default).toBe('allow')
    release()
    await oldRead
    expect(editor.panel.activeSessionId.value).toBe(2)
    expect(editor.panel.config.value.tools?.default).toBe('allow')
    expect(editor.panel.error.value).toBe('')
  })

  test('effective permissions follow the selected target and remembered-rule writes prevent leaving their scope', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'effective', sessionId: '1' })
    editor.setSessionPolicy('2', { path: { external: 'deny' }, tools: { default: 'ask' } })
    expect(await editor.navigateSession('2')).toBe(true)
    await editor.panel.load()
    expect(editor.panel.config.value).toEqual({ path: { external: 'deny' }, tools: { default: 'ask' } })
    expect(editor.panel.canEdit.value).toBe(false)
    editor.panel.rulesPanel.value = { refresh: async () => {}, busy: true }
    expect(await editor.navigateSession('1')).toBe(false)
    expect(await editor.navigate('access', 'global')).toBe(false)
    expect(editor.panel.activeSessionId.value).toBe(2)
    expect(editor.confirmations).toHaveLength(0)
  })

  test('a dirty settings pane does not intercept another pane target change or a focus transfer', async () => {
    const editor = await createPermissionEditor({}, undefined, {
      scope: 'session',
      sessionId: '1',
      windowId: 'pane-a',
      focused: false,
    })
    editor.panel.setToolDefault('deny')
    editor.setConfirmation(false)
    const paneB = {
      ...editor.route,
      query: { view: 'access', scope: 'session', targetSessionId: '2', windowId: 'pane-b' },
    }
    expect(await editor.guardRoute(paneB)).toBe(true)
    expect(await editor.guardRoute(paneB, { ...paneB, query: { ...paneB.query, targetSessionId: '1' } })).toBe(true)
    expect(await editor.guardLeave({ path: '/chat', query: { windowId: 'pane-b', sessionId: '3' } })).toBe(true)
    expect(
      await editor.guardRoute({ ...paneB, query: { view: 'access', scope: 'session', targetSessionId: '2' } }),
    ).toBe(true)
    expect(editor.panel.config.value.tools?.default).toBe('deny')
    expect(editor.panel.dirty.value).toBe(true)
    expect(editor.confirmations).toHaveLength(0)
  })

  test('entering a retained settings pane compares against its own target before allowing changes', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'session', sessionId: '1', windowId: 'pane-b' })
    editor.panel.setToolDefault('deny')
    editor.setConfirmation(false)
    const fromOtherPane = {
      ...editor.route,
      query: { view: 'access', scope: 'session', targetSessionId: '9', windowId: 'pane-a' },
    }
    expect(await editor.guardRoute({ ...editor.route, query: { ...editor.route.query } }, fromOtherPane)).toBe(true)
    expect(editor.confirmations).toHaveLength(0)
    expect(
      await editor.guardRoute(
        { ...editor.route, query: { ...editor.route.query, targetSessionId: '2' } },
        fromOtherPane,
      ),
    ).toBe(false)
    expect(editor.panel.activeSessionId.value).toBe(1)
    expect(editor.panel.config.value.tools?.default).toBe('deny')
    expect(editor.confirmations).toHaveLength(1)
  })

  test('the guard compares immutable route targets even if reactive presentation props have already advanced', async () => {
    const editor = await createPermissionEditor({}, undefined, { scope: 'session', sessionId: '1', windowId: 'pane-a' })
    editor.panel.setToolDefault('deny')
    editor.setConfirmation(false)
    const from = { ...editor.route, query: { ...editor.route.query } }
    editor.props.sessionId = '2'
    expect(
      await editor.guardRoute({ ...editor.route, query: { ...editor.route.query, targetSessionId: '2' } }, from),
    ).toBe(false)
    expect(editor.confirmations).toHaveLength(1)
  })
})
