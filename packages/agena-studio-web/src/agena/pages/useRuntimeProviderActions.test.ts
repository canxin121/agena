import { describe, expect, test } from 'bun:test'
import { ref } from 'vue'

import { useRuntimeProviderActions } from './useRuntimeProviderActions'

function createState() {
  const calls: string[] = []
  const state = {
    actionError: ref(''),
    actionMessage: ref(''),
    browserAuthCodeDrafts: {} as Record<string, string>,
    browserAuthInstanceDrafts: {} as Record<string, string>,
    browserAuthStartState: {} as Record<
      string,
      {
        provider_id: string
        authorize_url: string
        state: string
        pkce_verifier: string
        instance_url?: string | null
      } | null
    >,
    deviceAuthEnterpriseDrafts: {} as Record<string, string>,
    deviceAuthStartState: {} as Record<
      string,
      {
        provider_id: string
        verification_url: string
        user_code: string
        device_code: string
        interval_seconds: number
      } | null
    >,
    drafts: {
      anthropic: ' sk-ant-123 ',
      openai: ' ',
    } as Record<string, string>,
    load: async () => {
      calls.push('load')
    },
    openUrl: (url: string) => {
      calls.push(`openUrl:${url}`)
    },
    readRedirectUri: () => 'http://localhost:3210/auth/callback',
  }

  return { calls, state }
}

describe('useRuntimeProviderActions', () => {
  test('saveApiKey trims input, clears draft, and reloads', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeProviderActions(state, {
      deleteProviderCredential: async () => {},
      finishGitLabBrowserAuth: async () => ({ completed: true, provider: null }),
      finishOpenAiBrowserAuth: async () => ({ completed: true, provider: null }),
      pollAtomGitBrowserAuth: async () => ({ completed: true, provider: null }),
      pollCopilotDeviceAuth: async () => ({ completed: true, provider: null }),
      pollOpenAiDeviceAuth: async () => ({ completed: true, provider: null }),
      refreshProviderCredential: async () => {},
      setProviderApiKey: async (providerId, apiKey) => {
        calls.push(`setProviderApiKey:${providerId}:${apiKey}`)
      },
      startAtomGitBrowserAuth: async () => ({
        provider_id: 'atomgit',
        authorize_url: 'https://atomgit.com/oauth/authorize',
        state: 'atomgit-state',
        pkce_verifier: '',
      }),
      startCopilotDeviceAuth: async () => ({
        provider_id: 'github-copilot',
        verification_url: 'https://github.com/login/device',
        user_code: 'ABCD-EFGH',
        device_code: 'device-code',
        interval_seconds: 5,
      }),
      startGitLabBrowserAuth: async () => ({
        provider_id: 'gitlab',
        instance_url: 'https://gitlab.com',
        authorize_url: 'https://gitlab.com/oauth/authorize',
        state: 'state',
        pkce_verifier: 'pkce',
      }),
      startOpenAiBrowserAuth: async () => ({
        provider_id: 'openai',
        authorize_url: 'https://auth.openai.com/oauth/authorize',
        state: 'state',
        pkce_verifier: 'pkce',
      }),
      startOpenAiDeviceAuth: async () => ({
        provider_id: 'openai',
        verification_url: 'https://auth.openai.com/device',
        user_code: 'OPENAI-CODE',
        device_code: 'device-code',
        interval_seconds: 5,
      }),
    })

    await actions.saveApiKey('anthropic')
    await actions.saveApiKey('openai')

    expect(calls).toEqual(['setProviderApiKey:anthropic:sk-ant-123', 'load'])
    expect(state.drafts.anthropic).toBe('')
    expect(state.actionMessage.value).toBe('Saved API key for anthropic.')
  })

  test('clearCredential and refreshCredential reload state', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeProviderActions(state, {
      deleteProviderCredential: async (providerId) => {
        calls.push(`deleteProviderCredential:${providerId}`)
      },
      finishGitLabBrowserAuth: async () => ({ completed: true, provider: null }),
      finishOpenAiBrowserAuth: async () => ({ completed: true, provider: null }),
      pollAtomGitBrowserAuth: async () => ({ completed: true, provider: null }),
      pollCopilotDeviceAuth: async () => ({ completed: true, provider: null }),
      pollOpenAiDeviceAuth: async () => ({ completed: true, provider: null }),
      refreshProviderCredential: async (providerId) => {
        calls.push(`refreshProviderCredential:${providerId}`)
      },
      setProviderApiKey: async () => {},
      startAtomGitBrowserAuth: async () => ({
        provider_id: 'atomgit',
        authorize_url: 'https://atomgit.com/oauth/authorize',
        state: 'atomgit-state',
        pkce_verifier: '',
      }),
      startCopilotDeviceAuth: async () => ({
        provider_id: 'github-copilot',
        verification_url: 'https://github.com/login/device',
        user_code: 'ABCD-EFGH',
        device_code: 'device-code',
        interval_seconds: 5,
      }),
      startGitLabBrowserAuth: async () => ({
        provider_id: 'gitlab',
        instance_url: 'https://gitlab.com',
        authorize_url: 'https://gitlab.com/oauth/authorize',
        state: 'state',
        pkce_verifier: 'pkce',
      }),
      startOpenAiBrowserAuth: async () => ({
        provider_id: 'openai',
        authorize_url: 'https://auth.openai.com/oauth/authorize',
        state: 'state',
        pkce_verifier: 'pkce',
      }),
      startOpenAiDeviceAuth: async () => ({
        provider_id: 'openai',
        verification_url: 'https://auth.openai.com/device',
        user_code: 'OPENAI-CODE',
        device_code: 'device-code',
        interval_seconds: 5,
      }),
    })

    await actions.clearCredential('anthropic')
    await actions.refreshCredential('openai')

    expect(calls).toEqual(['deleteProviderCredential:anthropic', 'load', 'refreshProviderCredential:openai', 'load'])
    expect(state.actionMessage.value).toBe('Requested credential refresh for openai.')
  })

  test('browser and device auth flows update transient state and reload on completion', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeProviderActions(state, {
      deleteProviderCredential: async () => {},
      finishGitLabBrowserAuth: async ({ instanceUrl, code, pkceVerifier }) => {
        calls.push(`finishGitLabBrowserAuth:${instanceUrl}:${code}:${pkceVerifier}`)
        return { completed: true, provider: null }
      },
      finishOpenAiBrowserAuth: async ({ code, pkceVerifier, redirectUri }) => {
        calls.push(`finishOpenAiBrowserAuth:${code}:${pkceVerifier}:${redirectUri}`)
        return { completed: true, provider: null }
      },
      pollAtomGitBrowserAuth: async ({ providerId, state }) => {
        calls.push(`pollAtomGitBrowserAuth:${providerId}:${state}`)
        return { completed: true, provider: null }
      },
      pollCopilotDeviceAuth: async ({ deviceCode, enterpriseDomain }) => {
        calls.push(`pollCopilotDeviceAuth:${deviceCode}:${enterpriseDomain || ''}`)
        return { completed: true, provider: null }
      },
      pollOpenAiDeviceAuth: async ({ deviceCode, userCode }) => {
        calls.push(`pollOpenAiDeviceAuth:${deviceCode}:${userCode}`)
        return { completed: true, provider: null }
      },
      refreshProviderCredential: async () => {},
      setProviderApiKey: async () => {},
      startAtomGitBrowserAuth: async (providerId) => {
        calls.push(`startAtomGitBrowserAuth:${providerId}`)
        return {
          provider_id: providerId,
          authorize_url: 'https://atomgit.com/oauth/authorize',
          state: 'atomgit-state',
          pkce_verifier: '',
        }
      },
      startCopilotDeviceAuth: async (enterpriseDomain) => {
        calls.push(`startCopilotDeviceAuth:${enterpriseDomain || ''}`)
        return {
          provider_id: 'github-copilot',
          verification_url: 'https://github.com/login/device',
          user_code: 'ABCD-EFGH',
          device_code: 'copilot-device',
          interval_seconds: 5,
        }
      },
      startGitLabBrowserAuth: async ({ instanceUrl, redirectUri }) => {
        calls.push(`startGitLabBrowserAuth:${instanceUrl}:${redirectUri}`)
        return {
          provider_id: 'gitlab',
          instance_url: instanceUrl,
          authorize_url: 'https://gitlab.com/oauth/authorize',
          state: 'gitlab-state',
          pkce_verifier: 'gitlab-pkce',
        }
      },
      startOpenAiBrowserAuth: async (redirectUri) => {
        calls.push(`startOpenAiBrowserAuth:${redirectUri}`)
        return {
          provider_id: 'openai',
          authorize_url: 'https://auth.openai.com/oauth/authorize',
          state: 'openai-state',
          pkce_verifier: 'openai-pkce',
        }
      },
      startOpenAiDeviceAuth: async () => {
        calls.push('startOpenAiDeviceAuth')
        return {
          provider_id: 'openai',
          verification_url: 'https://auth.openai.com/device',
          user_code: 'OPENAI-CODE',
          device_code: 'openai-device',
          interval_seconds: 5,
        }
      },
    })

    state.browserAuthInstanceDrafts.gitlab = 'https://gitlab.example.com'
    await actions.startBrowserAuth('openai')
    await actions.startBrowserAuth('gitlab')
    await actions.startBrowserAuth('atomgit')
    expect(state.browserAuthStartState.openai?.pkce_verifier).toBe('openai-pkce')
    expect(state.browserAuthStartState.gitlab?.instance_url).toBe('https://gitlab.example.com')
    expect(state.browserAuthStartState.atomgit?.state).toBe('atomgit-state')

    state.browserAuthCodeDrafts.openai = 'openai-code'
    state.browserAuthCodeDrafts.gitlab = 'gitlab-code'
    await actions.finishBrowserAuth('openai')
    await actions.finishBrowserAuth('gitlab')
    await actions.finishBrowserAuth('atomgit')

    await actions.startDeviceAuth('openai')
    state.deviceAuthEnterpriseDrafts['github-copilot'] = 'github.example.com'
    await actions.startDeviceAuth('github-copilot')
    expect(state.deviceAuthStartState.openai?.user_code).toBe('OPENAI-CODE')
    expect(state.deviceAuthStartState['github-copilot']?.device_code).toBe('copilot-device')

    await actions.pollDeviceAuth('openai')
    await actions.pollDeviceAuth('github-copilot')

    expect(calls).toEqual([
      'startOpenAiBrowserAuth:http://localhost:3210/auth/callback',
      'openUrl:https://auth.openai.com/oauth/authorize',
      'startGitLabBrowserAuth:https://gitlab.example.com:http://localhost:3210/auth/callback',
      'openUrl:https://gitlab.com/oauth/authorize',
      'startAtomGitBrowserAuth:atomgit',
      'openUrl:https://atomgit.com/oauth/authorize',
      'finishOpenAiBrowserAuth:openai-code:openai-pkce:http://localhost:3210/auth/callback',
      'load',
      'finishGitLabBrowserAuth:https://gitlab.example.com:gitlab-code:gitlab-pkce',
      'load',
      'pollAtomGitBrowserAuth:atomgit:atomgit-state',
      'load',
      'startOpenAiDeviceAuth',
      'startCopilotDeviceAuth:github.example.com',
      'pollOpenAiDeviceAuth:openai-device:OPENAI-CODE',
      'load',
      'pollCopilotDeviceAuth:copilot-device:github.example.com',
      'load',
    ])
  })

  test('browser auth callback auto-finishes matching provider state', async () => {
    const { calls, state } = createState()
    const actions = useRuntimeProviderActions(state, {
      deleteProviderCredential: async () => {},
      finishGitLabBrowserAuth: async () => ({ completed: true, provider: null }),
      finishOpenAiBrowserAuth: async ({ code, pkceVerifier, redirectUri }) => {
        calls.push(`finishOpenAiBrowserAuth:${code}:${pkceVerifier}:${redirectUri}`)
        return { completed: true, provider: null }
      },
      pollAtomGitBrowserAuth: async () => ({ completed: true, provider: null }),
      pollCopilotDeviceAuth: async () => ({ completed: true, provider: null }),
      pollOpenAiDeviceAuth: async () => ({ completed: true, provider: null }),
      refreshProviderCredential: async () => {},
      setProviderApiKey: async () => {},
      startAtomGitBrowserAuth: async () => ({
        provider_id: 'atomgit',
        authorize_url: 'https://atomgit.com/oauth/authorize',
        state: 'atomgit-state',
        pkce_verifier: '',
      }),
      startCopilotDeviceAuth: async () => ({
        provider_id: 'github-copilot',
        verification_url: '',
        user_code: '',
        device_code: '',
        interval_seconds: 5,
      }),
      startGitLabBrowserAuth: async () => ({
        provider_id: 'gitlab',
        instance_url: 'https://gitlab.com',
        authorize_url: 'https://gitlab.com/oauth/authorize',
        state: 'gitlab-state',
        pkce_verifier: 'gitlab-pkce',
      }),
      startOpenAiBrowserAuth: async () => ({
        provider_id: 'openai',
        authorize_url: 'https://auth.openai.com/oauth/authorize',
        state: 'openai-state',
        pkce_verifier: 'openai-pkce',
      }),
      startOpenAiDeviceAuth: async () => ({
        provider_id: 'openai',
        verification_url: '',
        user_code: '',
        device_code: '',
        interval_seconds: 5,
      }),
    })

    state.browserAuthStartState.openai = {
      provider_id: 'openai',
      authorize_url: 'https://auth.openai.com/oauth/authorize',
      state: 'openai-state',
      pkce_verifier: 'openai-pkce',
    }

    await actions.handleBrowserAuthCallback({
      code: 'callback-code',
      state: 'openai-state',
    })

    expect(state.browserAuthCodeDrafts.openai).toBe('')
    expect(state.browserAuthStartState.openai === null).toBe(true)
    expect(calls).toEqual([
      'finishOpenAiBrowserAuth:callback-code:openai-pkce:http://localhost:3210/auth/callback',
      'load',
    ])
  })
})
