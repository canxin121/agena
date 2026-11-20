import type { PickerOption } from '@/components/ui/pickerOption.types'

export const PERMISSION_RULE_KINDS = ['filesystem', 'network', 'tool-name', 'command'] as const

export type PermissionRuleKind = (typeof PERMISSION_RULE_KINDS)[number]
export type PermissionRuleSource = 'local' | 'inherited' | 'effective'

/** A rendered rule. Each table has one fixed kind; the editor owns identity, inheritance and mutations. */
export type PermissionRuleRow = {
  key: string
  kind: PermissionRuleKind
  target: string
  condition: string
  mode: string
  modeLabel: string
  source: PermissionRuleSource
  sourceLabel: string
  readOnly?: boolean
  targetEditable?: boolean
  modeOptions?: PickerOption[]
  selectable?: boolean
  removable?: boolean
  editAction?: boolean
  sourceAction?: 'local' | 'inherit'
  sourceActionLabel?: string
  resetAction?: boolean
  resetActionLabel?: string
  networkAction?: 'allow' | 'deny'
}
