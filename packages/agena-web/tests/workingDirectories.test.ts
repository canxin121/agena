import { afterEach, beforeEach, describe, expect, test } from 'bun:test'

import {
  fetchSessionWorkingDirectories,
  normalizeWorkingDirectoriesView,
  parseDirsSlashArgs,
  registerSessionWorkingDirectory,
  removeSessionWorkingDirectory,
} from '../src/lib/workingDirectoriesApi'
import {
  CLIENT_COMMAND_ACTIONS,
  clientCommandFromCatalog,
  clientCommandsFromCatalog,
} from '../src/pages/chat/chatCommandsCatalog'

const originalFetch = globalThis.fetch

beforeEach(() => {
  globalThis.fetch = originalFetch
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('session working directories', () => {
  test('parses /dirs slash command list, add, shorthand, quoted, and remove forms', () => {
    expect(parseDirsSlashArgs('')).toEqual({ kind: 'list' })
    expect(parseDirsSlashArgs('list')).toEqual({ kind: 'list' })
    expect(parseDirsSlashArgs('ls')).toEqual({ kind: 'list' })
    expect(parseDirsSlashArgs('add ../shared')).toEqual({
      kind: 'add',
      path: '../shared',
      access: 'inherit',
    })
    expect(parseDirsSlashArgs('add ../shared ro')).toEqual({
      kind: 'add',
      path: '../shared',
      access: 'readonly',
    })
    expect(parseDirsSlashArgs('add "/tmp/my docs" readwrite')).toEqual({
      kind: 'add',
      path: '/tmp/my docs',
      access: 'readwrite',
    })
    expect(parseDirsSlashArgs('../extra rw')).toEqual({
      kind: 'add',
      path: '../extra',
      access: 'readwrite',
    })
    expect(parseDirsSlashArgs('remove "/tmp/my docs"')).toEqual({
      kind: 'remove',
      path: '/tmp/my docs',
    })
    expect(parseDirsSlashArgs('rm ../extra')).toEqual({
      kind: 'remove',
      path: '../extra',
    })
    expect(parseDirsSlashArgs('add ../extra invalid-mode')).toEqual({ kind: 'invalid' })
    expect(parseDirsSlashArgs('remove')).toEqual({ kind: 'invalid' })
    expect(parseDirsSlashArgs('add "unterminated')).toEqual({ kind: 'invalid' })
  })

  test('normalizes both detailed view and legacy array payloads', () => {
    expect(
      normalizeWorkingDirectoriesView({
        primary: '/workspace/main',
        directories: ['/workspace/docs', '/workspace/shared'],
        entries: [
          { path: '/workspace/docs', access: 'readonly' },
          { path: '/workspace/shared', access: 'readwrite' },
        ],
      }),
    ).toEqual({
      primary: '/workspace/main',
      directories: ['/workspace/docs', '/workspace/shared'],
      entries: [
        { path: '/workspace/docs', access: 'readonly' },
        { path: '/workspace/shared', access: 'readwrite' },
      ],
    })

    expect(normalizeWorkingDirectoriesView(['/workspace/legacy'])).toEqual({
      primary: null,
      directories: ['/workspace/legacy'],
      entries: [{ path: '/workspace/legacy', access: 'inherit' }],
    })
  })

  test('calls detailed GET, POST with access, and DELETE endpoints', async () => {
    const calls: Array<{ url: string; method: string; body?: string }> = []
    globalThis.fetch = (async (input, init) => {
      const url = String(input)
      const method = (init?.method || 'GET').toUpperCase()
      const body = typeof init?.body === 'string' ? init.body : undefined
      calls.push({ url, method, body })
      if (method === 'GET') {
        return new Response(
          JSON.stringify({
            primary: '/repo/main',
            directories: ['/repo/docs'],
            entries: [{ path: '/repo/docs', access: 'readonly' }],
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        )
      }
      if (method === 'POST') {
        return new Response(
          JSON.stringify({
            added: false,
            updated: true,
            directory: '/repo/docs',
            access: 'readwrite',
            directories: ['/repo/docs'],
            entries: [{ path: '/repo/docs', access: 'readwrite' }],
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        )
      }
      return new Response(
        JSON.stringify({
          removed: true,
          directory: '/repo/docs',
          directories: [],
          entries: [],
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      )
    }) as typeof fetch

    const view = await fetchSessionWorkingDirectories('42')
    expect(view.primary).toBe('/repo/main')
    expect(view.entries).toEqual([{ path: '/repo/docs', access: 'readonly' }])

    const registered = await registerSessionWorkingDirectory('42', '/repo/docs', 'readwrite')
    expect(registered.updated).toBe(true)
    expect(registered.access).toBe('readwrite')

    const removed = await removeSessionWorkingDirectory('42', '/repo/docs')
    expect(removed.removed).toBe(true)
    expect(removed.entries).toEqual([])

    expect(calls).toEqual([
      { url: '/api/v1/sessions/42/working-directories?detailed=true', method: 'GET', body: undefined },
      {
        url: '/api/v1/sessions/42/working-directories',
        method: 'POST',
        body: JSON.stringify({ path: '/repo/docs', access: 'readwrite' }),
      },
      {
        url: '/api/v1/sessions/42/working-directories',
        method: 'DELETE',
        body: JSON.stringify({ path: '/repo/docs' }),
      },
    ])
  })

  test('exposes /dirs as a project-only client command action', () => {
    expect(CLIENT_COMMAND_ACTIONS).toContain('dirs')
    const catalogEntry = {
      plugin_id: 'agena.commands',
      id: 'dirs',
      slash: '/dirs',
      aliases: ['/add-dir', '/working-dirs'],
      applies_to: 'project',
      accepts_empty_input: true,
      target: { kind: 'client', action: 'dirs' },
      docs: {
        usage: '[add <path> [readonly|readwrite|inherit] | remove <path>]',
        summary_key: 'command-dirs-summary',
      },
    }
    const parsed = clientCommandFromCatalog(catalogEntry as never)
    expect(parsed?.action).toBe('dirs')
    expect(parsed?.aliases).toEqual(['add-dir', 'working-dirs'])
    expect(clientCommandsFromCatalog([catalogEntry as never], false)).toHaveLength(1)
    expect(clientCommandsFromCatalog([catalogEntry as never], true)).toHaveLength(0)
  })
})
