export const PERMISSION_ACCESS_VIEW = 'access'
export const PERMISSION_ACCESS_SECTIONS = ['filesystem', 'network', 'tools', 'sandbox'] as const
export type PermissionAccessSection = (typeof PERMISSION_ACCESS_SECTIONS)[number]

export function permissionWorkspaceTarget(value: unknown): string {
  return String((Array.isArray(value) ? value[0] : value) ?? '').trim()
}

export function permissionAccessSection(value: unknown): PermissionAccessSection | undefined {
  const normalized = String(value || '')
    .trim()
    .toLowerCase()
  return PERMISSION_ACCESS_SECTIONS.find((section) => section === normalized)
}

export function isPermissionAccessView(value: unknown): boolean {
  const view = String(value || '')
    .trim()
    .toLowerCase()
  return view === PERMISSION_ACCESS_VIEW || view === 'overview' || Boolean(permissionAccessSection(view))
}

export function legacyPermissionAccessScope(view: unknown, scope: unknown): 'effective' | 'global' | undefined {
  if (view === 'overview') return 'effective'
  if (view === 'sandbox' && !scope) return 'global'
  return undefined
}

export function permissionAccessHash(section: unknown): string {
  const resolved = permissionAccessSection(section)
  return resolved ? `#permission-${resolved}` : ''
}

export function permissionSectionFromHash(hash: unknown): PermissionAccessSection | undefined {
  const value = String(hash || '')
  return value.startsWith('#permission-') ? permissionAccessSection(value.slice('#permission-'.length)) : undefined
}
