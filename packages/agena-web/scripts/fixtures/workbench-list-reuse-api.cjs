// Same-origin API fixtures for the prompt/settings list probe.
//
// Each factory returns an `async (route, url) => Promise<boolean>` callback
// that can be wrapped for `openPage(..., { api })`. It resolves `true` after
// fulfilling a route and `false` only when no fallback was provided for an
// unmatched route. Every factory accepts an optional `fallback` so a host
// runner can keep serving the rest of the application while reusing these
// simulated responses for the real Settings pages.
//
// The returned function also carries observable state:
//   api.requests, api.writes, api.state, api.reset()
function json(route, body, status = 200) {
  return route.fulfill({ status, json: body }).then(() => true)
}

function clone(value) {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value))
}

function attach(handle, state, requests, writes) {
  state.requests = requests
  state.writes = writes
  handle.requests = requests
  handle.writes = writes
  handle.state = state
  handle.reset = () => {
    requests.length = 0
    writes.length = 0
  }
  return handle
}

async function bodyOf(route) {
  try {
    return await route.request().postDataJSON()
  } catch {
    return undefined
  }
}

/* ------------------------------------------------------------------ prompts */

const fixtureBuiltins = [
  { id: 'agena.system.identity', title: 'Assistant identity', body: 'Identity rules.', when: 'always' },
  {
    id: 'agena.system.evidence',
    title: 'Runtime facts and evidence',
    body: 'Evidence rules.',
    when: 'user_facing',
  },
  {
    id: 'agena.profile.verification',
    title: 'Verification and evidence',
    body: 'Verification rules.',
    when: 'always',
    reusable: true,
  },
]

function fixtureSavedPrompt(id, title, category, body) {
  return { id, title, ...(category ? { category } : {}), body, when: { models: [], requires_tools: null } }
}

function defaultPromptState() {
  return {
    builtins: clone(fixtureBuiltins),
    global: {
      mode: 'append',
      order: ['agena.system.identity', 'agena.system.evidence'],
      disabled: [],
      inline: [],
      library: [
        fixtureSavedPrompt('saved.alpha', 'Alpha guidance', 'Guides', 'Alpha guidance body.'),
        fixtureSavedPrompt('saved.beta', 'Beta guidance', 'Guides', 'Beta guidance body.'),
        fixtureSavedPrompt('saved.gamma', 'Gamma checks', 'Runtime', 'Gamma checks body.'),
      ],
      models: [],
      categories: ['Guides', 'Runtime'],
    },
    workspace: {
      mode: 'append',
      order: [],
      disabled: [],
      inline: [],
      library: [fixtureSavedPrompt('saved.workspace', 'Workspace note', 'Workspace', 'Workspace note body.')],
      models: [],
      categories: ['Workspace'],
    },
    sessionPrompts: { mode: 'append', order: ['saved.alpha'], disabled: [], inline: [] },
    session: { id: 42, title: 'Fixture session', workspace_id: 1, state: 'active' },
    writes: [],
  }
}

/**
 * Settings sources, builtin prompts, prompt layers, and session prompt routes.
 * Covers SystemPromptPanel (global/workspace/session) and BotPromptEditor.
 */
function promptListsApi({ fallback, state: initial } = {}) {
  const state = { ...defaultPromptState(), ...clone(initial || {}) }
  const requests = []
  const writes = []

  const layerRead = (layer, key) => {
    const value = layer === 'workspace' ? state.workspace : state.global
    return {
      config_path: `fixture/${layer}.json`,
      config_found: true,
      revision: `prompts-${layer}-r1`,
      source: 'file',
      path: key,
      ...(layer === 'workspace' ? { workspace_id: 1 } : {}),
      value: clone(value),
      default_value: { mode: 'append', order: [], disabled: [], inline: [], library: [], models: [] },
    }
  }

  const bundle = () => {
    const global = layerRead('global', 'prompts')
    return {
      prompts: {
        effective: { ...clone(global), source: 'effective' },
        file: clone(global),
        global,
        workspace: layerRead('workspace', 'prompts'),
      },
    }
  }

  const preview = () => ({
    text: 'Identity rules.\n\nAlpha guidance body.',
    fragments: [
      {
        id: 'agena.system.identity',
        title: 'Assistant identity',
        scope: 'builtin',
        position: 1,
        bytes: 15,
        sha256: 'fixture-1',
        text: 'Identity rules.',
      },
      {
        id: 'saved.alpha',
        title: 'Alpha guidance',
        scope: 'global',
        position: 2,
        bytes: 21,
        sha256: 'fixture-2',
        text: 'Alpha guidance body.',
      },
    ],
    warnings: [],
    total_bytes: 36,
    over_budget: false,
  })

  const handle = async (route, url) => {
    const method = route.request().method()
    const { pathname } = url
    requests.push({ method, pathname, query: Object.fromEntries(url.searchParams) })
    if (pathname === '/api/v1/settings/sources') return json(route, bundle())
    if (pathname === '/api/v1/prompts/builtins') return json(route, state.builtins)
    if (pathname === '/api/v1/settings/layers/global') {
      const key = url.searchParams.get('path') || ''
      if (method === 'GET') return json(route, layerRead('global', key))
      if (method === 'PUT') {
        const body = await bodyOf(route)
        if (body?.path !== 'prompts') return json(route, { error: 'Unexpected settings path' }, 400)
        writes.push({ method, layer: 'global', ...body })
        state.global = clone(body.value)
        return json(route, { changed: true, current: clone(body.value), after_revision: 'prompts-global-r2' })
      }
    }
    if (pathname === '/api/v1/settings/layers/workspace') {
      const key = url.searchParams.get('path') || ''
      if (method === 'GET') return json(route, layerRead('workspace', key))
      if (method === 'PUT') {
        const body = await bodyOf(route)
        if (body?.path !== 'prompts') return json(route, { error: 'Unexpected settings path' }, 400)
        writes.push({ method, layer: 'workspace', ...body })
        state.workspace = clone(body.value)
        return json(route, { changed: true, current: clone(body.value), after_revision: 'prompts-workspace-r2' })
      }
    }
    const sessionState = pathname.match(/^\/api\/v1\/sessions\/(\d+)\/state$/)
    if (sessionState && method === 'GET') {
      return json(route, {
        session: clone(state.session),
        parts: [],
        queued_messages: [],
        queue_paused: false,
        execution: { prompts: clone(state.sessionPrompts) },
      })
    }
    const sessionPreview = pathname.match(/^\/api\/v1\/sessions\/(\d+)\/prompts\/preview$/)
    if (sessionPreview && method === 'GET') return json(route, preview())
    // Feeds the Settings session picker: a roots-only list plus the selected
    // session's read-back.
    if (pathname === '/api/v1/sessions' && method === 'GET') {
      const search = (url.searchParams.get('search') || '').trim().toLowerCase()
      const items = [state.session].filter((entry) => !search || entry.title.toLowerCase().includes(search))
      return json(route, {
        items: clone(items),
        total: items.length,
        page: { has_more: false, next_cursor: null },
      })
    }
    const session = pathname.match(/^\/api\/v1\/sessions\/(\d+)$/)
    if (session && method === 'GET') return json(route, clone(state.session))
    if (session && method === 'PUT') {
      const body = await bodyOf(route)
      writes.push({ method, layer: 'session', sessionId: Number(session[1]), ...body })
      if (body && body.prompts) state.sessionPrompts = clone(body.prompts)
      return json(route, { ...clone(state.session), ...(body || {}) })
    }
    if (fallback) return fallback(route, url)
    // Nothing owns the route: letting the caller abort makes the failure loud.
    return false
  }
  return attach(handle, state, requests, writes)
}

/* ------------------------------------------------------------ session storage */

const storageBots = [
  {
    id: 'bot-1',
    name: 'Support bot',
    session_count: 30,
    session_bytes: 3000,
    profile_bytes: 100,
    channel_bytes: 50,
    channel_files: 10,
  },
  {
    id: 'bot-2',
    name: 'Docs bot',
    session_count: 15,
    session_bytes: 1200,
    profile_bytes: 0,
    channel_bytes: 0,
    channel_files: 0,
  },
]

const storageDatabase = { file_bytes: 100_000, wal_bytes: 500, reusable_bytes: 1_000, can_reclaim: true }
const storageShared = { workspace_files: 100, web: 0, memory: 0, logs: 50, plugins: 0 }

function storageSummary(index) {
  const number = index + 1
  return {
    session_id: 100 + number,
    title: `Session ${String(number).padStart(2, '0')}`,
    workspace_id: 1,
    workspace_name: 'Fixture workspace',
    bot_id: number % 2 === 0 ? 'bot-1' : null,
    bot_name: number % 2 === 0 ? 'Support bot' : null,
    surface: number % 2 === 0 ? 'group' : 'local',
    deleted: false,
    busy: false,
    busy_reason: null,
    modified_at_ms: 1_700_000_000_000 + number * 1_000,
    bytes: {
      conversation: number * 100,
      recordings: number * 10,
      media: 0,
      files: number * 5,
      cache: number === 1 ? 4_096 : 0,
      plugins: 0,
    },
  }
}

const storageFiles = (sessionId, index) => ({
  id: `file-${sessionId}-${index + 1}`,
  name: `artifact-${String(index + 1).padStart(2, '0')}.bin`,
  kind: index % 3 === 0 ? 'upload' : index % 3 === 1 ? 'artifact' : 'cache',
  bytes: 100 + index,
  protection: index === 1 ? 'shared' : null,
})

function totalOf(bytes) {
  return bytes.conversation + bytes.recordings + bytes.media + bytes.files + bytes.cache + bytes.plugins
}

/**
 * `/api/v1/session-storage`, per-session detail/file paging, preview, cleanup,
 * and reclaim-database routes for SessionStoragePanel.
 */
function sessionStorageApi({ fallback, count = 45 } = {}) {
  const sessions = Array.from({ length: count }, (_, index) => storageSummary(index))
  const requests = []
  const writes = []
  const cleanups = []
  const operations = new Map()
  const state = {
    sessions,
    cleanups,
    listRequests: [],
    detailRequests: [],
    database: clone(storageDatabase),
  }

  const filtered = (query) => {
    const scope = query.get('scope') || ''
    const bot = query.get('bot') || ''
    const search = (query.get('search') || '').trim().toLowerCase()
    return sessions.filter((session) => {
      if (scope === 'project' && session.bot_id) return false
      if (scope === 'bot' && !session.bot_id) return false
      if (bot && session.bot_id !== bot) return false
      if (search && !session.title.toLowerCase().includes(search)) return false
      return true
    })
  }

  const detail = (session, fileOffset, fileLimit) => {
    const fileCount = session.session_id === 101 ? 17 : 2
    const files = Array.from({ length: fileCount }, (_, index) => storageFiles(session.session_id, index)).slice(
      fileOffset,
      fileOffset + fileLimit,
    )
    return {
      ...session,
      shared_history: false,
      files,
      file_count: fileCount,
      removable_recording_bytes: session.session_id === 101 ? 2_048 : 0,
      operation: operations.get(session.session_id)?.current ?? null,
    }
  }

  const preview = (selection) => ({
    token: `cleanup-token-${selection.files ? selection.files.join('-') : selection.purge ? 'purge' : 'cache'}`,
    file_bytes: selection.files ? 512 : 0,
    database_bytes: selection.purge || selection.cache ? 4_096 : selection.recordings ? 1_024 : 0,
    file_count: selection.files ? 1 : 0,
    keeps_shared_history: false,
    protection: null,
  })

  const handle = async (route, url) => {
    const method = route.request().method()
    const { pathname } = url
    const entry = { method, pathname, query: Object.fromEntries(url.searchParams) }
    requests.push(entry)
    if (pathname === '/api/v1/session-storage' && method === 'GET') {
      state.listRequests.push(entry)
      const offset = Number(url.searchParams.get('offset') || 0)
      const limit = Number(url.searchParams.get('limit') || 20)
      const items = filtered(url.searchParams)
      const page = items.slice(offset, offset + limit)
      return json(route, {
        items: page,
        total: items.length,
        measured_at_ms: 1_700_000_000_000,
        bytes: items.reduce((sum, session) => sum + totalOf(session.bytes), 0),
        bots: clone(storageBots),
        database: clone(state.database),
        shared: clone(storageShared),
      })
    }
    if (pathname === '/api/v1/session-storage/reclaim-database' && method === 'POST') {
      state.database = { ...clone(storageDatabase), can_reclaim: false }
      return json(route, { database: clone(state.database), released_bytes: 12_345 })
    }
    const storage = pathname.match(/^\/api\/v1\/sessions\/(\d+)\/storage(?:\/(operation|preview|cleanup))?$/)
    if (storage) {
      const session = sessions.find((item) => item.session_id === Number(storage[1]))
      const action = storage[2]
      if (!session) return json(route, { error: 'Unknown session' }, 404)
      if (action === 'operation' && method === 'GET') {
        const operation = operations.get(session.session_id)
        if (!operation) return json(route, null)
        operation.current = operation.completed
        return json(route, operation.current)
      }
      if (action === 'preview' && method === 'POST') {
        // previewSessionStorage posts the selection itself; confirmSessionStorage
        // wraps it as { selection, token }.
        const body = await bodyOf(route)
        return json(route, preview(body?.selection || body || {}))
      }
      if (action === 'cleanup' && method === 'POST') {
        const body = await bodyOf(route)
        const result = preview(body?.selection || {})
        if (body?.token !== result.token) return json(route, { error: 'Incorrect preview token' }, 409)
        cleanups.push({ sessionId: session.session_id, ...body, result })
        const completed = {
          id: 'cleanup-1',
          state: 'completed',
          purge: Boolean(body.selection?.purge),
          files_removed: result.file_count,
          files_skipped: 0,
          file_bytes_removed: result.file_bytes,
          database_bytes_removed: result.database_bytes,
          keeps_shared_history: false,
          detail: null,
        }
        const current = {
          ...completed,
          state: 'running',
          files_removed: 0,
          file_bytes_removed: 0,
          database_bytes_removed: 0,
        }
        operations.set(session.session_id, { current, completed })
        return json(route, current)
      }
      if (!action && method === 'GET') {
        state.detailRequests.push(entry)
        const fileOffset = Number(url.searchParams.get('file_offset') || 0)
        const fileLimit = Number(url.searchParams.get('file_limit') || 15)
        return json(route, detail(session, fileOffset, fileLimit))
      }
    }
    if (fallback) return fallback(route, url)
    // Nothing owns the route: letting the caller abort makes the failure loud.
    return false
  }
  return attach(handle, state, requests, writes)
}

/* ---------------------------------------------------------- channel assignments */

const channelBots = [
  {
    id: '11111111-1111-4111-8111-111111111111',
    name: 'Support bot',
    local_session_id: 5,
    archived: false,
    persona: { instructions: '' },
    revision: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
  },
  {
    id: '22222222-2222-4222-8222-222222222222',
    name: 'Docs bot',
    local_session_id: 6,
    archived: false,
    persona: { instructions: '' },
    revision: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
  },
]

const channelInventory = {
  platforms: [
    {
      platform: 'telegram',
      title: 'Telegram',
      conversation_count: 3,
      conversation_counts: { direct: 1, group: 1, broadcast: 1 },
      accounts: [
        {
          account: 'acct-1',
          tenant: 'tenant-1',
          binding_id: 'binding-telegram',
          selector: 'fixture',
          state: 'enabled',
          conversations: [
            { kind: 'group', resource: 'room-alpha', room_title: 'Alpha room' },
            { kind: 'direct', resource: 'room-beta', room_title: 'Beta DM' },
            { kind: 'broadcast', resource: 'room-gamma' },
          ],
        },
      ],
    },
  ],
}

const channelParticipants = {
  'room-alpha': [
    { actor: 'ada', display_name: 'Ada Lovelace', first_seen_at_ms: 1, last_seen_at_ms: 10 },
    { actor: 'bob', display_name: 'Bob Stone', first_seen_at_ms: 2, last_seen_at_ms: 20 },
    { actor: 'cleo', display_name: 'Cleo Park', first_seen_at_ms: 3, last_seen_at_ms: 30 },
  ],
  'room-beta': [{ actor: 'dana', display_name: 'Dana Wells', first_seen_at_ms: 4, last_seen_at_ms: 40 }],
  'room-gamma': [{ actor: 'erin', display_name: 'Erin Cole', first_seen_at_ms: 5, last_seen_at_ms: 50 }],
}

const defaultAccessControl = {
  profiles: {
    '@default': { name: 'Default permissions', description: '', revision: 1, permission: {} },
  },
  users: [],
  conversations: [],
}

function defaultAccessControlCatalog() {
  return {
    profiles: {
      '@default': { name: 'Default permissions', description: '', revision: 1, permission: {} },
      operator: { name: 'Operator', description: 'Can run tools', revision: 1, permission: {} },
      reviewer: { name: 'Reviewer', description: 'Read only', revision: 2, permission: {} },
    },
    users: [
      {
        platform: 'telegram',
        tenant: 'tenant-1',
        account: 'acct-1',
        actor: 'ada',
        resource: 'room-alpha',
        profile_id: 'reviewer',
      },
    ],
    conversations: [
      {
        platform: 'telegram',
        tenant: 'tenant-1',
        account: 'acct-1',
        resource: 'room-alpha',
        default_profile_id: 'operator',
      },
    ],
  }
}

/**
 * Bots, platform inventory, participant pages, and the global access_control
 * layer used by ChannelAssignmentsPanel (and PermissionProfilesPanel).
 *
 * Participant pagination: the group room returns Ada and Bob first with a
 * cursor, then Cleo. `search` filters server-side.
 */
function channelAssignmentsApi({ fallback } = {}) {
  const requests = []
  const writes = []
  const state = {
    bots: clone(channelBots),
    inventory: clone(channelInventory),
    participants: clone(channelParticipants),
    accessControl: defaultAccessControlCatalog(),
    defaultAccessControl: clone(defaultAccessControl),
    revision: 'access-r1',
  }

  const accessRead = () => ({
    config_path: 'fixture/global.json',
    config_found: true,
    revision: state.revision,
    source: 'file',
    path: 'access_control',
    value: clone(state.accessControl),
    default_value: clone(state.defaultAccessControl),
  })

  const participantsFor = (url, resource) => {
    const search = (url.searchParams.get('search') || '').trim().toLowerCase()
    const after = url.searchParams.get('after')
    const all = state.participants[resource] || []
    const matching = search
      ? all.filter(
          (participant) =>
            participant.actor.toLowerCase().includes(search) ||
            (participant.display_name || '').toLowerCase().includes(search),
        )
      : all
    // Only the unfiltered group room is paginated: two entries, then a cursor.
    if (resource === 'room-alpha' && !search) {
      if (after === 'alpha-page-2') return { participants: matching.slice(2), next_after: null }
      return { participants: matching.slice(0, 2), next_after: 'alpha-page-2' }
    }
    return { participants: matching, next_after: null }
  }

  const handle = async (route, url) => {
    const method = route.request().method()
    const { pathname } = url
    const entry = { method, pathname, query: Object.fromEntries(url.searchParams) }
    requests.push(entry)
    if (pathname === '/api/v1/bots' && method === 'GET') {
      return json(route, { items: clone(state.bots), default_bot_id: state.bots[0].id })
    }
    if (/^\/api\/v1\/sessions\/\d+\/platforms$/.test(pathname) && method === 'GET') {
      return json(route, clone(state.inventory))
    }
    const participants = pathname.match(/^\/api\/v1\/sessions\/\d+\/integrations\/([^/]+)\/participants$/)
    if (participants && method === 'GET') {
      const resource = url.searchParams.get('resource') || ''
      return json(route, participantsFor(url, resource))
    }
    // GET carries ?path=access_control; setRuntimeSetting sends a PUT with no
    // query and puts the path in the JSON body.
    if (pathname === '/api/v1/settings/layers/global') {
      if (method === 'GET' && url.searchParams.get('path') === 'access_control') return json(route, accessRead())
      if (method === 'PUT') {
        const body = await bodyOf(route)
        if (body?.path !== 'access_control') return json(route, { error: 'Unexpected settings path' }, 400)
        writes.push({ method, layer: 'global', ...body })
        state.accessControl = clone(body.value)
        state.revision = 'access-r2'
        return json(route, {
          changed: true,
          before_revision: 'access-r1',
          after_revision: state.revision,
          current: clone(state.accessControl),
        })
      }
    }
    if (fallback) return fallback(route, url)
    // Nothing owns the route: letting the caller abort makes the failure loud.
    return false
  }
  return attach(handle, state, requests, writes)
}

module.exports = {
  channelAssignmentsApi,
  channelBots,
  channelInventory,
  fixtureBuiltins,
  promptListsApi,
  sessionStorageApi,
  storageBots,
}
