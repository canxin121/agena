import type { Ref } from 'vue'

import type { AuthBrowserStartResponse, AuthDeviceStartResponse, AuthProvider } from '../lib/agenaApi'

export type SettingsProvidersStateInput = {
  authProviders: Ref<AuthProvider[]>
  browserAuthCodeDrafts: Record<string, string>
  browserAuthInstanceDrafts: Record<string, string>
  browserAuthStartState: Record<string, AuthBrowserStartResponse | null>
  deviceAuthEnterpriseDrafts: Record<string, string>
  deviceAuthStartState: Record<string, AuthDeviceStartResponse | null>
  drafts: Record<string, string>
  finishBrowserAuth: (providerId: string) => void | Promise<void>
  pollDeviceAuth: (providerId: string) => void | Promise<void>
  saveApiKey: (providerId: string) => void | Promise<void>
  refreshCredential: (providerId: string) => void | Promise<void>
  clearCredential: (providerId: string) => void | Promise<void>
  startBrowserAuth: (providerId: string) => void | Promise<void>
  startDeviceAuth: (providerId: string) => void | Promise<void>
}

export function useSettingsProvidersState(input: SettingsProvidersStateInput) {
  return {
    authProviders: input.authProviders,
    browserAuthCodeDrafts: input.browserAuthCodeDrafts,
    browserAuthInstanceDrafts: input.browserAuthInstanceDrafts,
    browserAuthStartState: input.browserAuthStartState,
    deviceAuthEnterpriseDrafts: input.deviceAuthEnterpriseDrafts,
    deviceAuthStartState: input.deviceAuthStartState,
    drafts: input.drafts,
    finishBrowserAuth: input.finishBrowserAuth,
    pollDeviceAuth: input.pollDeviceAuth,
    saveApiKey: input.saveApiKey,
    refreshCredential: input.refreshCredential,
    clearCredential: input.clearCredential,
    startBrowserAuth: input.startBrowserAuth,
    startDeviceAuth: input.startDeviceAuth,
  }
}
