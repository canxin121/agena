import { migrateLegacyStudioStorage } from './legacyStudio'

const STORAGE_NAMESPACE = 'agena-web'

migrateLegacyStudioStorage()

function namespacedKey(path: string): string {
  return `${STORAGE_NAMESPACE}.${path}`
}

export const localStorageKeys = {
  auth: {
    uiTokenByBaseUrl: namespacedKey('auth.ui-token-by-base-url.v1'),
  },
  backends: {
    configV1: namespacedKey('backends.v1'),
  },
} as const

export const sessionStorageKeys = {
  auth: {
    authRequired: namespacedKey('auth.required'),
  },
} as const
