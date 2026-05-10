import type { Ref } from 'vue'

import {
  deleteProviderCredential,
  refreshProviderCredential,
  setProviderApiKey,
} from '../lib/agenaApi'

export type RuntimeProviderActionsInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  drafts: Record<string, string>
  load: () => Promise<void>
}

export type RuntimeProviderActionsDeps = {
  deleteProviderCredential: typeof deleteProviderCredential
  refreshProviderCredential: typeof refreshProviderCredential
  setProviderApiKey: typeof setProviderApiKey
}

const defaultDeps: RuntimeProviderActionsDeps = {
  deleteProviderCredential,
  refreshProviderCredential,
  setProviderApiKey,
}

export function useRuntimeProviderActions(
  input: RuntimeProviderActionsInput,
  deps: RuntimeProviderActionsDeps = defaultDeps,
) {
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

  return {
    clearCredential,
    refreshCredential,
    saveApiKey,
  }
}
