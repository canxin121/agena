import type { Ref } from 'vue'

import type { ConfigSettingsReadResponse } from '../lib/agenaApi'

export type SettingsPermissionsStateInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  permissionConfig: Ref<ConfigSettingsReadResponse | null>
}

export function useSettingsPermissionsState(input: SettingsPermissionsStateInput) {
  return {
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    load: input.load,
    permissionConfig: input.permissionConfig,
  }
}
