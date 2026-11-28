import { afterEach, beforeEach, describe, expect, it } from 'bun:test'

import {
  invalidateNativeSessionStatusCache,
  manageNativeAgentSession,
} from '@/features/sessions/api/nativeAgentSessions'

const originalFetch = globalThis.fetch

function jsonResponse(payload: unknown): Response {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  })
}

const sampleStatus = {
  kind: 'status',
  session: { native_id: 'nat-1', cwd: '/repo', title: 'Hello', updated_at: null },
  native_status: null,
  additional_directories: [],
  capabilities: {
    status: false,
    title: false,
    close: true,
    delete: false,
    fork: false,
    additional_directories: true,
    system_prompt: true,
  },
}

describe('manageNativeAgentSession status deduplication', () => {
  beforeEach(() => {
    invalidateNativeSessionStatusCache()
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
    invalidateNativeSessionStatusCache()
  })

  it('coalesces concurrent status calls for the same session and reuses short cache', async () => {
    let calls = 0
    let resolveFetch!: (res: Response) => void
    globalThis.fetch = ((() => {
      calls++
      return new Promise<Response>((resolve) => {
        resolveFetch = resolve
      })
    }) as unknown) as typeof fetch

    const p1 = manageNativeAgentSession('acp:10', { action: 'status' })
    const p2 = manageNativeAgentSession('acp:10', { action: 'status' })
    expect(calls).toBe(1)

    resolveFetch(jsonResponse(sampleStatus))
    const [r1, r2] = await Promise.all([p1, p2])
    expect(r1).toEqual(sampleStatus as never)
    expect(r2).toEqual(sampleStatus as never)

    // Subsequent immediate call hits the 2s short cache.
    const r3 = await manageNativeAgentSession('acp:10', { action: 'status' })
    expect(r3).toEqual(sampleStatus as never)
    expect(calls).toBe(1)
  })

  it('isolates caller abort signals and invalidates status cache on mutations', async () => {
    let calls = 0
    let resolveFetch!: (res: Response) => void
    globalThis.fetch = ((() => {
      calls++
      if (calls === 1) {
        return new Promise<Response>((resolve) => {
          resolveFetch = resolve
        })
      }
      if (calls === 2) {
        return Promise.resolve(jsonResponse({ kind: 'title', title: 'Renamed' }))
      }
      return Promise.resolve(jsonResponse(sampleStatus))
    }) as unknown) as typeof fetch

    const ac1 = new AbortController()
    const p1 = manageNativeAgentSession('acp:11', { action: 'status' }, ac1.signal)
    const p2 = manageNativeAgentSession('acp:11', { action: 'status' })

    ac1.abort()
    await expect(p1).rejects.toThrow(/aborted/i)

    resolveFetch(jsonResponse(sampleStatus))
    expect(await p2).toEqual(sampleStatus as never)
    expect(calls).toBe(1)

    // Mutation clears the short cache so next status call hits fetch again.
    await manageNativeAgentSession('acp:11', { action: 'set_title', title: 'Renamed' })
    expect(calls).toBe(2)

    await manageNativeAgentSession('acp:11', { action: 'status' })
    expect(calls).toBe(3)
  })
})
