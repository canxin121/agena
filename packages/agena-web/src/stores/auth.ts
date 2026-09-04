import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { ApiError, apiJson, apiUrl } from '../lib/api'
import {
  buildActiveUiAuthHeaders,
  clearUiAuthTokenForBaseUrl,
  readUiAuthTokenVersion,
  writeUiAuthTokenForBaseUrl,
} from '../lib/uiAuthToken'
import { readActiveBackendBaseUrl } from '../lib/backend'

const AUTH_REQUEST_TIMEOUT_MS = 5000

function timeoutSignal(ms: number): AbortSignal | undefined {
  try {
    if (typeof AbortSignal !== 'undefined' && typeof AbortSignal.timeout === 'function') {
      return AbortSignal.timeout(ms)
    }
  } catch {
    // ignore
  }
  return undefined
}

type AuthStatusOk = { authenticated: boolean; disabled?: boolean; locked?: boolean; token?: string }
type ApiProblemBody = {
  problem?: {
    user?: { fallback?: string }
  }
}

export const useAuthStore = defineStore('auth', () => {
  const checked = ref(false)
  const authenticated = ref(false)
  const locked = ref(false)
  const disabled = ref(false)
  const lastError = ref<string | null>(null)
  const loginInFlight = ref(false)

  // Every operation that changes the auth state advances this revision. A
  // refresh captures both this revision and the token version at request time;
  // a late response from an older auth state is then ignored instead of
  // overwriting a successful login.
  let stateRevision = 0
  let refreshInFlight: {
    stateRevision: number
    tokenVersion: number
    promise: Promise<void>
  } | null = null

  const needsLogin = computed(() => checked.value && !disabled.value && locked.value)

  function requireLogin() {
    // A request from the old page can finish while the user is submitting the
    // password. It must not tear down the login attempt in progress.
    if (loginInFlight.value) return false

    stateRevision += 1
    // Force the app into the locked state immediately (e.g. when an API call returns auth.required).
    checked.value = true
    authenticated.value = false
    disabled.value = false
    locked.value = true
    lastError.value = null

    // Clear any stored token for the active backend so we don't keep sending a stale credential.
    try {
      clearUiAuthTokenForBaseUrl(readActiveBackendBaseUrl())
    } catch {
      // ignore
    }

    return true
  }

  async function performRefresh(requestRevision: number, requestTokenVersion: number) {
    const isCurrent = () =>
      requestRevision === stateRevision && requestTokenVersion === readUiAuthTokenVersion() && !loginInFlight.value

    try {
      const authHeaders = buildActiveUiAuthHeaders()
      const resp = await fetch(apiUrl('/auth/session'), {
        signal: timeoutSignal(AUTH_REQUEST_TIMEOUT_MS),
        headers: {
          accept: 'application/json',
          ...authHeaders,
        },
        credentials: authHeaders.authorization ? 'omit' : 'include',
      })

      // Do not let an old refresh clear or relock a session that changed while
      // this request was in flight (most commonly during password login).
      if (!isCurrent()) return

      lastError.value = null
      checked.value = true
      if (resp.ok) {
        const data = (await resp.json()) as AuthStatusOk
        if (!isCurrent()) return
        authenticated.value = Boolean(data.authenticated)
        disabled.value = Boolean(data.disabled)
        // The server reports a missing UI session as HTTP 200 with
        // { authenticated: false, locked: true }. Treat that as a login
        // requirement instead of mounting the protected page and waiting for
        // its first request to return 401.
        locked.value = !disabled.value && Boolean(data.locked)

        // Best-effort: if the backend returns a token (optional), persist it.
        const token = typeof data.token === 'string' ? data.token.trim() : ''
        if (token) {
          writeUiAuthTokenForBaseUrl(readActiveBackendBaseUrl(), token)
        }
        return
      }

      if (resp.status === 401 || resp.status === 429) {
        const data = (await resp.json().catch(() => null)) as ApiProblemBody | null
        if (!isCurrent()) return
        authenticated.value = false
        disabled.value = false
        locked.value = true
        lastError.value = String(
          data?.problem?.user?.fallback ||
            (resp.status === 429 ? 'Too many login attempts' : 'UI authentication is required.'),
        )
        clearUiAuthTokenForBaseUrl(readActiveBackendBaseUrl())
        return
      }

      const txt = await resp.text().catch(() => '')
      if (!isCurrent()) return
      lastError.value = txt || `Auth status failed (${resp.status})`
      authenticated.value = false
      disabled.value = false
      locked.value = false
    } catch (err) {
      if (!isCurrent()) return
      checked.value = true
      lastError.value = err instanceof Error ? err.message : String(err)
      authenticated.value = false
      disabled.value = false
      locked.value = false
    }
  }

  function refresh() {
    const requestRevision = stateRevision
    const requestTokenVersion = readUiAuthTokenVersion()
    const existing = refreshInFlight
    if (existing && existing.stateRevision === requestRevision && existing.tokenVersion === requestTokenVersion) {
      return existing.promise
    }

    const promise = performRefresh(requestRevision, requestTokenVersion)
    const current = { stateRevision: requestRevision, tokenVersion: requestTokenVersion, promise }
    refreshInFlight = current
    void promise.then(
      () => {
        if (refreshInFlight === current) refreshInFlight = null
      },
      () => {
        if (refreshInFlight === current) refreshInFlight = null
      },
    )
    return promise
  }

  async function login(password: string) {
    if (loginInFlight.value) return

    stateRevision += 1
    const requestRevision = stateRevision
    loginInFlight.value = true
    lastError.value = null
    try {
      const data = await apiJson<AuthStatusOk>('/auth/session', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ password }),
      })

      if (requestRevision !== stateRevision) return

      const token = typeof data?.token === 'string' ? data.token.trim() : ''
      if (token) {
        writeUiAuthTokenForBaseUrl(readActiveBackendBaseUrl(), token)
      }

      // The create-session response is authoritative and already contains the
      // newly issued token. Avoid a second refresh here: a pre-login refresh
      // may still be returning a 401 and must not race this success path.
      checked.value = true
      authenticated.value = Boolean(data?.authenticated)
      disabled.value = Boolean(data?.disabled)
      locked.value = !authenticated.value && !disabled.value
      lastError.value = null
    } catch (err) {
      if (requestRevision !== stateRevision) return

      if (err instanceof ApiError) {
        lastError.value = err.message || err.bodyText || null
      } else {
        lastError.value = err instanceof Error ? err.message : String(err)
      }
      checked.value = true
      authenticated.value = false
      disabled.value = false
      locked.value = true
    } finally {
      loginInFlight.value = false
    }
  }

  return {
    checked,
    authenticated,
    loginInFlight,
    lastError,
    needsLogin,
    refresh,
    login,
    requireLogin,
  }
})
