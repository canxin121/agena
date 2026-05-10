import type { Ref } from 'vue'

import type { AuthProvider } from '../lib/agenaApi'

export type SettingsProvidersStateInput = {
  authProviders: Ref<AuthProvider[]>
  drafts: Record<string, string>
  saveApiKey: (providerId: string) => void | Promise<void>
  refreshCredential: (providerId: string) => void | Promise<void>
  clearCredential: (providerId: string) => void | Promise<void>
}

export function useSettingsProvidersState(input: SettingsProvidersStateInput) {
  return {
    authProviders: input.authProviders,
    drafts: input.drafts,
    saveApiKey: input.saveApiKey,
    refreshCredential: input.refreshCredential,
    clearCredential: input.clearCredential,
  }
}
