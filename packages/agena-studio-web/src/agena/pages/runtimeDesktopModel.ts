import type {
  DesktopBackendStatus,
  DesktopConfig,
  DesktopRuntimeInfo,
  DesktopUpdateProgress,
} from '@/lib/desktopConfig'

export type DesktopFact = {
  label: string
  value: string
  mono?: boolean
}

export function buildDesktopConfigFacts(config: DesktopConfig | null): DesktopFact[] {
  if (!config) return []
  return [
    { label: 'Autostart', value: config.autostart_on_boot ? 'enabled' : 'disabled' },
    { label: 'Host', value: config.backend.host || '127.0.0.1', mono: true },
    { label: 'Port', value: String(config.backend.port) },
    { label: 'Workspace Root', value: config.backend.workspace_root || 'n/a', mono: true },
    { label: 'Database Path', value: config.backend.database_path || 'n/a', mono: true },
    { label: 'Database URL', value: config.backend.database_url || 'n/a', mono: true },
    { label: 'Log Level', value: config.backend.backend_log_level || 'default' },
    { label: 'UI Cookie SameSite', value: config.backend.ui_cookie_samesite || 'default' },
  ]
}

export function buildDesktopStatusFacts(status: DesktopBackendStatus | null): DesktopFact[] {
  if (!status) return []
  return [
    { label: 'Running', value: status.running ? 'yes' : 'no' },
    { label: 'URL', value: status.url || 'n/a', mono: true },
    { label: 'Last Error', value: status.last_error || 'none' },
    { label: 'Error Code', value: status.last_error_info?.code || 'n/a', mono: true },
    { label: 'Error Summary', value: status.last_error_info?.summary || 'n/a' },
    { label: 'Hint', value: status.last_error_info?.hint || 'n/a' },
  ]
}

export function buildDesktopRuntimeFacts(info: DesktopRuntimeInfo | null): DesktopFact[] {
  if (!info) return []
  return [
    { label: 'Installer Version', value: info.installerVersion },
    { label: 'Target', value: info.installerTarget, mono: true },
    { label: 'Channel', value: info.installerChannel },
    { label: 'Type', value: info.installerType },
    { label: 'Manager', value: info.installerManager },
  ]
}

export function buildDesktopUpdateFacts(progress: DesktopUpdateProgress | null): DesktopFact[] {
  if (!progress) return []
  return [
    { label: 'Running', value: progress.running ? 'yes' : 'no' },
    { label: 'Kind', value: progress.kind || 'n/a' },
    { label: 'Phase', value: progress.phase || 'n/a' },
    { label: 'Message', value: progress.message || 'n/a' },
    {
      label: 'Progress',
      value: progress.totalBytes ? `${progress.downloadedBytes} / ${progress.totalBytes}` : String(progress.downloadedBytes),
      mono: true,
    },
    { label: 'Error', value: progress.error || 'none' },
  ]
}
