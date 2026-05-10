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
  type AuthBrowserStartResponse,
  type AuthDeviceStartResponse,
} from '../lib/agenaApi'

export type RuntimeProviderActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
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
  function readProviderIdFromBrowserState(stateValue: string): string | null {
    const state = String(stateValue || '').trim()
    if (!state) return null
    for (const [providerId, start] of Object.entries(input.browserAuthStartState)) {
      if (start?.state === state) return providerId
    }
    return null
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
      input.actionError.value = err instanceof Error ? err.message : String(err)
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
      input.actionError.value = err instanceof Error ? err.message : String(err)
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
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function startBrowserAuth(providerId: string) {
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      if (providerId === 'openai') {
        const start = await deps.startOpenAiBrowserAuth(input.readRedirectUri())
        input.browserAuthStartState[providerId] = start
        input.openUrl(start.authorize_url)
        input.actionMessage.value = `Opened browser login for ${providerId}.`
      } else if (providerId === 'gitlab') {
        const instanceUrl = String(input.browserAuthInstanceDrafts[providerId] || '').trim() || 'https://gitlab.com'
        const start = await deps.startGitLabBrowserAuth({
          instanceUrl,
          redirectUri: input.readRedirectUri(),
        })
        input.browserAuthStartState[providerId] = start
        input.openUrl(start.authorize_url)
        input.actionMessage.value = `Opened browser login for ${providerId}.`
      }
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function finishBrowserAuth(providerId: string) {
    const start = input.browserAuthStartState[providerId]
    const code = String(input.browserAuthCodeDrafts[providerId] || '').trim()
    if (!start || !code) return
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      if (providerId === 'openai') {
        await deps.finishOpenAiBrowserAuth({
          code,
          pkceVerifier: start.pkce_verifier,
          redirectUri: input.readRedirectUri(),
        })
      } else if (providerId === 'gitlab') {
        await deps.finishGitLabBrowserAuth({
          instanceUrl: start.instance_url || String(input.browserAuthInstanceDrafts[providerId] || '').trim() || 'https://gitlab.com',
          code,
          pkceVerifier: start.pkce_verifier,
          redirectUri: input.readRedirectUri(),
        })
      }
      input.browserAuthCodeDrafts[providerId] = ''
      input.browserAuthStartState[providerId] = null
      input.actionMessage.value = `Completed browser login for ${providerId}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function handleBrowserAuthCallback(inputValue: {
    code?: string
    state?: string
    error?: string
  }) {
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
      if (providerId === 'openai') {
        input.deviceAuthStartState[providerId] = await deps.startOpenAiDeviceAuth()
      } else if (providerId === 'github-copilot') {
        input.deviceAuthStartState[providerId] = await deps.startCopilotDeviceAuth(
          input.deviceAuthEnterpriseDrafts[providerId],
        )
      }
      input.actionMessage.value = `Started device login for ${providerId}.`
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function pollDeviceAuth(providerId: string) {
    const start = input.deviceAuthStartState[providerId]
    if (!start) return
    input.actionMessage.value = ''
    input.actionError.value = ''
    try {
      const result =
        providerId === 'openai'
          ? await deps.pollOpenAiDeviceAuth({
              deviceCode: start.device_code,
              userCode: start.user_code,
            })
          : await deps.pollCopilotDeviceAuth({
              deviceCode: start.device_code,
              enterpriseDomain: input.deviceAuthEnterpriseDrafts[providerId],
            })
      if (result.completed) {
        input.deviceAuthStartState[providerId] = null
        input.actionMessage.value = `Completed device login for ${providerId}.`
        await input.load()
        return
      }
      input.actionMessage.value = `Device login for ${providerId} is still pending.`
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
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
