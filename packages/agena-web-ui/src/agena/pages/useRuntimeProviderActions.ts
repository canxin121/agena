import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import { setAuthCallbackHandler } from '../lib/authCallbackRegistry'
import {
  deleteProviderCredential,
  finishGitLabBrowserAuth,
  finishOpenAiBrowserAuth,
  pollCopilotDeviceAuth,
  pollOpenAiDeviceAuth,
  refreshProviderCredential,
  setProviderApiKey,
  startCopilotDeviceAuth,
  startGitLabBrowserAuth,
  startOpenAiBrowserAuth,
  startOpenAiDeviceAuth,
  type AuthProvider,
  type AuthBrowserStartResponse,
  type AuthDeviceStartResponse,
} from '../lib/agenaApi'

export type RuntimeProviderActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  authProviders: Ref<AuthProvider[]>
  browserAuthCodeDrafts: Record<string, string>
  browserAuthInstanceDrafts: Record<string, string>
  browserAuthStartState: Record<string, AuthBrowserStartResponse | null>
  deviceAuthStartState: Record<string, AuthDeviceStartResponse | null>
  deviceAuthEnterpriseDrafts: Record<string, string>
  drafts: Record<string, string>
  load: () => Promise<void>
  openUrl: (url: string) => void
  readRedirectUri: () => string
}

export type RuntimeProviderActionsDeps = {
  deleteProviderCredential: typeof deleteProviderCredential
  finishGitLabBrowserAuth: typeof finishGitLabBrowserAuth
  finishOpenAiBrowserAuth: typeof finishOpenAiBrowserAuth
  pollCopilotDeviceAuth: typeof pollCopilotDeviceAuth
  pollOpenAiDeviceAuth: typeof pollOpenAiDeviceAuth
  refreshProviderCredential: typeof refreshProviderCredential
  setProviderApiKey: typeof setProviderApiKey
  startCopilotDeviceAuth: typeof startCopilotDeviceAuth
  startGitLabBrowserAuth: typeof startGitLabBrowserAuth
  startOpenAiBrowserAuth: typeof startOpenAiBrowserAuth
  startOpenAiDeviceAuth: typeof startOpenAiDeviceAuth
}

const defaultDeps: RuntimeProviderActionsDeps = {
  deleteProviderCredential,
  finishGitLabBrowserAuth,
  finishOpenAiBrowserAuth,
  pollCopilotDeviceAuth,
  pollOpenAiDeviceAuth,
  refreshProviderCredential,
  setProviderApiKey,
  startCopilotDeviceAuth,
  startGitLabBrowserAuth,
  startOpenAiBrowserAuth,
  startOpenAiDeviceAuth,
}

export function useRuntimeProviderActions(
  input: RuntimeProviderActionsInput,
  deps: RuntimeProviderActionsDeps = defaultDeps,
) {
  function findProvider(providerId: string) {
    return input.authProviders.value.find((provider) => provider.provider_id === providerId) || null
  }

  function readBrowserLoginKind(providerId: string) {
    return findProvider(providerId)?.browser_login_kind || null
  }

  function readDeviceLoginKind(providerId: string) {
    return findProvider(providerId)?.device_login_kind || null
  }

  function readProviderIdFromBrowserState(stateValue: string): string | null {
    const state = String(stateValue || '').trim()
    if (!state) return null
    for (const [providerId, start] of Object.entries(input.browserAuthStartState)) {
      if (start?.state === state) return providerId
    }
    return null
  }

  function parseBrowserCallbackInput(inputValue: string, expectedState: string): { code: string } {
    const trimmed = String(inputValue || '').trim()
    if (!trimmed) {
      throw new Error('Paste the callback URL or authorization code first.')
    }

    const directCode = !trimmed.includes('://') && !trimmed.includes('?') && !trimmed.includes('code=')
    if (directCode) {
      return { code: trimmed }
    }

    const rawUrl = trimmed.startsWith('?')
      ? `/auth/callback${trimmed}`
      : trimmed.startsWith('code=')
        ? `/auth/callback?${trimmed}`
        : trimmed
    const baseUrl =
      typeof window !== 'undefined' && window.location?.origin ? window.location.origin : 'http://localhost'
    let parsed: URL
    try {
      parsed = new URL(rawUrl, baseUrl)
    } catch {
      throw new Error('Callback input must be a full callback URL, query string, or raw authorization code.')
    }

    const error = parsed.searchParams.get('error')?.trim()
    if (error) {
      const detail = parsed.searchParams.get('error_description')?.trim()
      throw new Error(detail || error)
    }

    const code = parsed.searchParams.get('code')?.trim()
    const state = parsed.searchParams.get('state')?.trim()
    if (!code) {
      throw new Error('OAuth callback is missing the code parameter.')
    }
    if (state && state !== expectedState) {
      throw new Error('OAuth callback state does not match the pending login session.')
    }
    return { code }
  }

  async function saveApiKey(providerId: string) {
    const apiKey = String(input.drafts[providerId] || '').trim()
    if (!apiKey) return
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      await deps.setProviderApiKey(providerId, apiKey)
      input.drafts[providerId] = ''
      input.actionMessage.value = `Saved API key for ${providerId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function clearCredential(providerId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      await deps.deleteProviderCredential(providerId)
      input.actionMessage.value = `Cleared credential for ${providerId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function refreshCredential(providerId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      await deps.refreshProviderCredential(providerId)
      input.actionMessage.value = `Requested credential refresh for ${providerId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function startBrowserAuth(providerId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const browserLoginKind = readBrowserLoginKind(providerId)
      if (browserLoginKind === 'openai_chatgpt') {
        const start = await deps.startOpenAiBrowserAuth({
          providerId,
          redirectUri: input.readRedirectUri(),
        })
        input.browserAuthStartState[providerId] = start
        input.openUrl(start.authorize_url)
        input.actionMessage.value = `Opened browser login for ${providerId}.`
      } else if (browserLoginKind === 'gitlab') {
        const start = await deps.startGitLabBrowserAuth({
          providerId,
          redirectUri: input.readRedirectUri(),
        })
        input.browserAuthInstanceDrafts[providerId] = start.instance_url || ''
        input.browserAuthStartState[providerId] = start
        input.openUrl(start.authorize_url)
        input.actionMessage.value = `Opened browser login for ${providerId}.`
      } else {
        throw new Error(`${providerId} does not support browser login.`)
      }
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function finishBrowserAuth(providerId: string) {
    const start = input.browserAuthStartState[providerId]
    if (!start) return
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const browserLoginKind = readBrowserLoginKind(providerId)
      const { code } = parseBrowserCallbackInput(input.browserAuthCodeDrafts[providerId], start.state)
      if (browserLoginKind === 'openai_chatgpt') {
        await deps.finishOpenAiBrowserAuth({
          providerId,
          code,
          pkceVerifier: start.pkce_verifier,
          redirectUri: input.readRedirectUri(),
        })
      } else if (browserLoginKind === 'gitlab') {
        await deps.finishGitLabBrowserAuth({
          providerId,
          code,
          pkceVerifier: start.pkce_verifier,
          redirectUri: input.readRedirectUri(),
        })
      } else {
        throw new Error(`${providerId} does not support browser login.`)
      }
      input.browserAuthCodeDrafts[providerId] = ''
      input.browserAuthStartState[providerId] = null
      input.actionMessage.value = `Completed browser login for ${providerId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function handleBrowserAuthCallback(inputValue: { code?: string; state?: string; error?: string }) {
    const error = String(inputValue.error || '').trim()
    if (error) {
      input.actionError.value = error
      return
    }

    const providerId = readProviderIdFromBrowserState(String(inputValue.state || ''))
    const code = String(inputValue.code || '').trim()
    if (!providerId || !code) return
    input.browserAuthCodeDrafts[providerId] = code
    await finishBrowserAuth(providerId)
  }

  setAuthCallbackHandler((payload) => {
    void handleBrowserAuthCallback(payload)
  })

  async function startDeviceAuth(providerId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const deviceLoginKind = readDeviceLoginKind(providerId)
      if (deviceLoginKind === 'openai_chatgpt') {
        input.deviceAuthStartState[providerId] = await deps.startOpenAiDeviceAuth({ providerId })
      } else if (deviceLoginKind === 'github_copilot') {
        input.deviceAuthStartState[providerId] = await deps.startCopilotDeviceAuth({
          providerId,
          enterpriseDomain: input.deviceAuthEnterpriseDrafts[providerId],
        })
      } else {
        throw new Error(`${providerId} does not support device login.`)
      }
      input.actionMessage.value = `Started device login for ${providerId}.`
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  async function pollDeviceAuth(providerId: string) {
    const start = input.deviceAuthStartState[providerId]
    if (!start) return
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const deviceLoginKind = readDeviceLoginKind(providerId)
      const result =
        deviceLoginKind === 'openai_chatgpt'
          ? await deps.pollOpenAiDeviceAuth({
              providerId,
              deviceCode: start.device_code,
              userCode: start.user_code,
            })
          : deviceLoginKind === 'github_copilot'
            ? await deps.pollCopilotDeviceAuth({
                providerId,
                deviceCode: start.device_code,
                enterpriseDomain: input.deviceAuthEnterpriseDrafts[providerId],
              })
            : (() => {
                throw new Error(`${providerId} does not support device login.`)
              })()
      if (result.completed) {
        input.deviceAuthStartState[providerId] = null
        input.actionMessage.value = `Completed device login for ${providerId}.`
        await input.load()
        return
      }
      input.actionMessage.value = `Device login for ${providerId} is still pending.`
    } catch (err) {
      input.actionError.value = userErrorMessage(err)
    }
  }

  return {
    clearCredential,
    finishBrowserAuth,
    handleBrowserAuthCallback,
    pollDeviceAuth,
    refreshCredential,
    saveApiKey,
    startBrowserAuth,
    startDeviceAuth,
  }
}
