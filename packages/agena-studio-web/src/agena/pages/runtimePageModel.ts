import type { PluginStatus, RuntimeStatus } from '@/agena/lib/agenaApi'

export type OperatorCard = {
  label: string
  value: string
}

export function buildOperatorCards(runtime: RuntimeStatus | null): OperatorCard[] {
  if (!runtime) return []
  return [
    { label: 'Generation', value: String(runtime.generation) },
    { label: 'Providers', value: String(runtime.provider_ids.length) },
    { label: 'Plugins', value: String(runtime.plugin_count) },
    { label: 'MCP Servers', value: String(runtime.operator.mcp.server_count) },
    { label: 'LSP Servers', value: String(runtime.operator.lsp.server_count) },
    { label: 'Skills', value: String(runtime.operator.skills.skill_count) },
  ]
}

export function pickNextPluginId(currentPluginId: string, plugins: PluginStatus[]): string {
  if (currentPluginId && plugins.some((plugin) => plugin.plugin_id === currentPluginId)) {
    return currentPluginId
  }
  return plugins[0]?.plugin_id || ''
}
