import { settingsText as st } from '@/i18n/settingsText'

/** Localized label for a plugin lifecycle state; unknown values stay verbatim. */
export function pluginStateLabel(state: string | null | undefined): string {
  const raw = String(state ?? '').trim()
  switch (raw.toLowerCase()) {
    case 'running':
      return st('Running')
    case 'restarting':
      return st('Restarting')
    case 'failed':
      return st('Failed')
    case 'stopped':
      return st('Stopped')
    default:
      return raw
  }
}
