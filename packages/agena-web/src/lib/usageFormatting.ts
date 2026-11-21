const units = [
  [1e18, 'E'],
  [1e15, 'P'],
  [1e12, 't'],
  [1e9, 'b'],
  [1e6, 'm'],
  [1e3, 'k'],
] as const

const integer = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 })
const decimal = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 })
const percent = new Intl.NumberFormat(undefined, { style: 'percent', maximumFractionDigits: 1 })

export function formatUsageCount(value?: number | null, locale?: string): string {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return '0'
  const unit = units.find(([divisor]) => value >= divisor - Math.floor(divisor / 20_000))
  const maximumFractionDigits = unit ? 1 : 0
  const formatter = locale ? new Intl.NumberFormat(locale, { maximumFractionDigits }) : unit ? decimal : integer
  return unit ? `${formatter.format(value / unit[0])}${unit[1]}` : formatter.format(value)
}

export function exactUsageCount(value?: number | null): string | undefined {
  if (typeof value === 'number' && value >= 1_000 && Number.isSafeInteger(value)) return integer.format(value)
}

export function formatUsagePercent(value?: number | null): string {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return '—'
  return percent.format(Math.min(value, 1))
}
