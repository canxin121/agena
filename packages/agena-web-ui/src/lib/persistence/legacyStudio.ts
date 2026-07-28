/** One-time migration from the retired Studio/browser storage namespaces. */

const LEGACY_NAMESPACES = ['agena-studio.', 'agena.'] as const
const CANONICAL_NAMESPACE = 'agena-web.'
const MIGRATION_MARKER = `${CANONICAL_NAMESPACE}legacy-migration-v1`

function migrateStorage(storage: Storage | undefined): void {
  if (!storage) return
  try {
    if (storage.getItem(MIGRATION_MARKER) === 'done') return
    const keys = Array.from({ length: storage.length }, (_, index) => storage.key(index)).filter((key): key is string =>
      Boolean(key),
    )
    for (const key of keys) {
      const legacyNamespace = LEGACY_NAMESPACES.find((namespace) => key.startsWith(namespace))
      if (!legacyNamespace) continue
      const canonicalKey = `${CANONICAL_NAMESPACE}${key.slice(legacyNamespace.length)}`
      if (storage.getItem(canonicalKey) === null) {
        const value = storage.getItem(key)
        if (value !== null) storage.setItem(canonicalKey, value)
      }
    }
    storage.setItem(MIGRATION_MARKER, 'done')
  } catch {
    // Storage can be unavailable or quota-restricted; active paths remain usable.
  }
}

export function migrateLegacyStudioStorage(): void {
  if (typeof window === 'undefined') return
  migrateStorage(window.localStorage)
  migrateStorage(window.sessionStorage)
}
