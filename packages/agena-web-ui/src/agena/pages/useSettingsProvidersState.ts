import type { Ref } from 'vue'

import type {
  AuthBrowserStartResponse,
  AuthDeviceStartResponse,
  AuthProvider,
  ConfigSettingsReadResponse,
  ModelCatalogEntry,
  ProviderModel,
  ProviderSummary,
} from '../lib/agenaApi'

export type SettingsProvidersStateInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  authProviders: Ref<AuthProvider[]>
  browserAuthCodeDrafts: Record<string, string>
  browserAuthInstanceDrafts: Record<string, string>
  browserAuthStartState: Record<string, AuthBrowserStartResponse | null>
  deviceAuthEnterpriseDrafts: Record<string, string>
  deviceAuthStartState: Record<string, AuthDeviceStartResponse | null>
  drafts: Record<string, string>
  catalogEntries: Ref<ModelCatalogEntry[]>
  permissionConfig: Ref<ConfigSettingsReadResponse | null>
  load: () => Promise<void>
  providerModels: Record<string, ProviderModel[]>
  providers: Ref<ProviderSummary[]>
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
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    authProviders: input.authProviders,
    browserAuthCodeDrafts: input.browserAuthCodeDrafts,
    browserAuthInstanceDrafts: input.browserAuthInstanceDrafts,
    browserAuthStartState: input.browserAuthStartState,
    deviceAuthEnterpriseDrafts: input.deviceAuthEnterpriseDrafts,
    deviceAuthStartState: input.deviceAuthStartState,
    drafts: input.drafts,
    catalogEntries: input.catalogEntries,
    permissionConfig: input.permissionConfig,
    load: input.load,
    providerModels: input.providerModels,
    providers: input.providers,
    finishBrowserAuth: input.finishBrowserAuth,
    pollDeviceAuth: input.pollDeviceAuth,
    saveApiKey: input.saveApiKey,
    refreshCredential: input.refreshCredential,
    clearCredential: input.clearCredential,
    startBrowserAuth: input.startBrowserAuth,
    startDeviceAuth: input.startDeviceAuth,
  }
}
