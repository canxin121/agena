export type ThinkingModeShape = {
  default?: boolean
  display_name?: string | null
  description?: string | null
  preset?: string | null
  thinking?: {
    type?: string | null
    effort?: string | null
    budget_tokens?: number | null
  } | null
  strategy?: string | null
  effort?: string | null
  budget_tokens?: number | null
  disabled?: boolean
}

export type ModeWithSelector = { mode: ThinkingModeShape; selector: string }

const STRENGTH: Record<string, number> = {
  off: 0,
  disabled: 0,
  no: 0,
  none: 0,
  minimal: 1,
  low: 2,
  medium: 3,
  high: 4,
  xhigh: 5,
  max: 6,
  maximum: 6,
}

function stringValue(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

export function thinkingModeSelector(mode: ThinkingModeShape | null | undefined): string {
  if (!mode) return ''
  const preset = stringValue(mode.preset)
  if (preset) return preset
  const type = stringValue(mode.thinking?.type || mode.strategy).toLowerCase()
  if (type === 'disabled' || mode.disabled === true) return 'off'
  if (type === 'effort' || type === 'adaptive') return stringValue(mode.thinking?.effort || mode.effort)
  return ''
}

function selectorStrength(selector: string): number | null {
  for (const token of selector.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean)) {
    const tier = STRENGTH[token]
    if (tier !== undefined) return tier
  }
  return null
}

function modeStrength({ mode, selector }: ModeWithSelector): [number, number] {
  const type = stringValue(mode.thinking?.type || mode.strategy).toLowerCase()
  const effort = stringValue(mode.thinking?.effort || mode.effort).toLowerCase()
  if (type === 'disabled' || mode.disabled === true) return [0, 0]
  if ((type === 'effort' || type === 'adaptive') && effort) {
    return [STRENGTH[effort] ?? selectorStrength(selector) ?? 3, 0]
  }
  if (type === 'budget') {
    const budget = Number(mode.thinking?.budget_tokens ?? mode.budget_tokens)
    return [selectorStrength(selector) ?? 3, Number.isFinite(budget) ? Math.max(0, budget) : 0]
  }
  return [selectorStrength(selector) ?? 3, 0]
}

export function compareThinkingModes(left: ModeWithSelector, right: ModeWithSelector): number {
  const [leftTier, leftBudget] = modeStrength(left)
  const [rightTier, rightBudget] = modeStrength(right)
  return (
    leftTier - rightTier ||
    leftBudget - rightBudget ||
    (left.selector < right.selector ? -1 : left.selector > right.selector ? 1 : 0)
  )
}

export function sortedThinkingModeNames(value: Record<string, unknown>): string[] {
  return Object.entries(value)
    .filter(([name]) => name !== 'default')
    .map(([selector, raw]) => {
      const rawMode = record(raw)
      const thinking = record(rawMode.thinking)
      const mode: ThinkingModeShape = {
        preset: selector,
        strategy: stringValue(rawMode.strategy),
        effort: stringValue(rawMode.effort),
        budget_tokens: typeof rawMode.budget_tokens === 'number' ? rawMode.budget_tokens : undefined,
        disabled: rawMode.disabled === true,
        thinking: {
          type: stringValue(thinking.type),
          effort: stringValue(thinking.effort),
          budget_tokens: typeof thinking.budget_tokens === 'number' ? thinking.budget_tokens : undefined,
        },
      }
      return { selector, mode }
    })
    .sort((left, right) =>
      compareThinkingModes(
        { selector: left.selector, mode: left.mode },
        { selector: right.selector, mode: right.mode },
      ),
    )
    .map(({ selector }) => selector)
}

export function orderedThinkingModeMap(value: unknown): unknown {
  const modes = record(value)
  if (!Object.keys(modes).length) return value
  const names = sortedThinkingModeNames(modes)
  const ordered: Record<string, unknown> = {}
  if (Object.prototype.hasOwnProperty.call(modes, 'default')) ordered.default = modes.default
  for (const name of names) ordered[name] = modes[name]
  return ordered
}
