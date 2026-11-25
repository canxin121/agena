const project = '/fixture'
const fileName = 'workspace-layout-' + 'long-file-name-'.repeat(4) + '.gif'
const terminalPath = '/api/v1/workbench/terminal'
const terminalStateEvents = `${terminalPath}/state/events`
const previewPath = '/api/v1/workbench/preview/sessions'
const gif = Buffer.from('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7', 'base64')

function sessionLabels(kind) {
  return Array.from(
    { length: 20 },
    (_, index) => `${kind}-${String(index + 1).padStart(2, '0')}${index === 0 ? '-long-session-name'.repeat(3) : ''}`,
  )
}

function initialTerminalState() {
  const labels = sessionLabels('terminal')
  const sessionIds = labels.map((_, index) => `term-${index + 1}`)
  return {
    version: 1,
    updatedAt: 1,
    activeSessionId: sessionIds[0],
    sessionIds,
    sessionMetaById: Object.fromEntries(sessionIds.map((id, index) => [id, { name: labels[index] }])),
    folders: [],
  }
}

function writeTerminalStateStream(response) {
  // Keep the snapshot stream open as the real service would; a finite mocked
  // response would display a disconnection error immediately after loading.
  response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' })
  response.write(`data: ${JSON.stringify({ type: 'terminal-ui-state.snapshot', state: initialTerminalState() })}\n\n`)
}

function workspaceApi(root = project) {
  const previews = sessionLabels('preview')
  let terminalState = initialTerminalState()
  const changes = [{ path: 'changed.txt', index: ' ', workingDir: 'M', insertions: 1, deletions: 0 }]
  const changePage = { files: changes, totalFiles: 1, offset: 0, limit: 30, hasMore: false }
  const status = {
    ...changePage,
    current: 'main',
    tracking: null,
    ahead: 0,
    behind: 0,
    stagedCount: 0,
    mergeCount: 0,
    unstagedCount: 1,
    untrackedCount: 0,
    scopePages: { unstaged: changePage },
  }
  const git = {
    check: { isGitRepository: true },
    repos: { base: root, count: 1, repos: [{ root, relative: '.' }] },
    status,
    'remote-info': { remotes: [] },
    'signing-info': {},
    state: {},
    branches: { all: ['main'], current: 'main', branches: {} },
    tags: { tags: [] },
    log: {
      commits: [{ hash: 'fixture-commit-a1b2c3', shortHash: 'a1b2c3', subject: 'Fixture history commit', parents: [] }],
      hasMore: false,
    },
    'commit-files': { files: [{ path: 'changed.txt', status: 'M', insertions: 1, deletions: 0 }] },
    diff: {
      diff: 'diff --git a/changed.txt b/changed.txt\n--- a/changed.txt\n+++ b/changed.txt\n@@ -1 +1 @@\n-before\n+after\n',
    },
    'commit-diff': { diff: '' },
  }
  const assistant = {
    source_session_id: 42,
    profile: {
      workspace_id: 1,
      revision: 3,
      preferences: {
        timezone: 'UTC',
        quiet_start_minute: 0,
        quiet_end_minute: 60,
        daily_limit: 2,
        cooldown_seconds: 900,
        proactive_enabled: true,
        memory_enabled: true,
      },
    },
    memories: [],
    reminders: [],
  }
  return async (route, url) => {
    const { pathname } = url
    const method = route.request().method()
    const json = (body) => route.fulfill({ json: body })
    if (pathname === '/api/v1/health') return json({ status: 'ok', database_connected: true })
    if (pathname === '/auth/session') return json({ authenticated: true, disabled: true })
    if (/\/(events|watch|stream)$/.test(pathname)) {
      return route.fulfill({ contentType: 'text/event-stream', body: ': fixture\n\n' })
    }
    if (pathname === `${terminalPath}/state`) {
      if (method === 'PUT') terminalState = { ...route.request().postDataJSON(), version: terminalState.version + 1 }
      return json(terminalState)
    }
    if (method !== 'GET') return route.fulfill({ status: 405 })
    if (pathname === '/api/v1/settings' && url.searchParams.get('path') === 'ui.transcript') return json({ value: {} })
    if (pathname === '/api/v1/sessions' || pathname === '/api/v1/bots') {
      return json({ items: [], total: 0, page: { has_more: false } })
    }
    if (pathname === '/api/v1/sessions/42') {
      return json({ id: 42, title: 'Fixture session', workspace_id: 1, state: 'active' })
    }
    if (pathname.startsWith(`${terminalPath}/term-`)) {
      return json({ sessionId: pathname.split('/').at(-1), cwd: root, running: false })
    }
    if (pathname === previewPath) {
      return json({
        sessions: previews.map((id) => ({
          id,
          directory: root,
          runDirectory: root,
          state: 'stopped',
          command: 'fixture',
          args: [],
          targetUrl: 'http://127.0.0.1:4000',
          logsPath: '.agena/preview.log',
        })),
      })
    }
    if (pathname === '/api/v1/workbench/fs/home') return json({ home: root })
    if (pathname === '/api/v1/workbench/fs/list') {
      const directory = url.searchParams.get('path')
      return json({
        path: directory,
        entries:
          directory === root && root
            ? [{ path: `${root}/${fileName}`, name: fileName, isFile: true, isDirectory: false }]
            : [],
        hasMore: false,
      })
    }
    if (pathname.startsWith('/api/v1/workbench/fs/') && url.searchParams.get('path')?.endsWith('.gif')) {
      return route.fulfill({ contentType: 'image/gif', body: gif })
    }
    const gitAction = pathname.match(/^\/api\/v1\/workbench\/git\/(.+)$/)?.[1]
    if (gitAction && Object.hasOwn(git, gitAction)) return json(git[gitAction])
    if (pathname === '/api/v1/plugins/surface') return json({ permission_tools: [], activity_kinds: [] })
    if (pathname === '/api/v1/sessions/42/assistant') return json(assistant)
    return route.fulfill({ status: 404, json: { message: `No fixture for ${pathname}` } })
  }
}

module.exports = {
  fileName,
  project,
  sessionLabels,
  terminalPath,
  terminalStateEvents,
  previewPath,
  workspaceApi,
  writeTerminalStateStream,
}
